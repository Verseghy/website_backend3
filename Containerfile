# Containerfile for the backend3 production image.
#
# Only Pipelines-as-Code builds this file, on pushes to master
# (.tekton/push.yaml, ADR 0022 section 1). GitHub Actions never builds it, so a
# PR that changes this file, rust-toolchain.toml, admin/.node-version or a base
# image is built locally with `podman build` by its author, and the PR body
# says so (ADR 0022, Consequences):
#
#   podman build --build-arg SOURCE_COMMIT="$(git rev-parse HEAD)" -t backend3:local .
#
# Rules, and where each one comes from:
#   - Three stages (ADR 0004 section 2): `admin` builds admin/dist with
#     Node.js, `rust` builds the binary, `runtime` ships both. The rust stage
#     does not depend on the admin stage: the binary reads admin/dist from disk
#     at run time and does not embed it.
#   - Base images are UBI 9 only, never UBI 10 or a RHEL 10 userland, because
#     the hub's build nodes lack x86-64-v3 (ADR 0022 section 2). Every base is
#     pinned by digest (the manifest-list digest, which `skopeo inspect`
#     reports). Bump one on purpose with
#       skopeo inspect --no-tags docker://registry.access.redhat.com/ubi9/<name>:latest | jq -r .Digest
#   - NODE_VERSION equals admin/.node-version and RUST_VERSION equals the
#     channel in rust-toolchain.toml. The "Toolchain" CI job
#     (scripts/ci/check-toolchain.sh) checks both, and the base-image pins.
#   - The Rust toolchain comes from rustup in a UBI 9 stage, because the
#     builder cannot pull ubi9/rust-toolset (ADR 0022, Context). rustup-init is
#     pinned by version and SHA-256; rustup then verifies every toolchain
#     component it downloads.
#   - `cargo build --release --locked`, with no cargo-chef: the vfs builder
#     keeps no layer cache between pushes, so chef would only add its own
#     compile time. The target is the default x86_64-unknown-linux-gnu and
#     RUSTFLAGS stay empty, never `-C target-cpu=native`: the binary must run
#     on the edge node, not only on the machine that built it.
#   - No SQLite and no OpenSSL are linked (ADR 0022 sections 2 and 4), so the
#     rust stage needs only gcc as the linker and C compiler for build scripts.
#   - The runtime image runs as an arbitrary non-root UID under the
#     restricted-v2 SCC: every file is owned by root:0 and world-readable, the
#     binary is world-executable, and nothing is written at run time except
#     the pod's emptyDir for temporary files and the uploads volume, so
#     readOnlyRootFilesystem works (ADR 0023 section 8).
#   - Default command `start --server-and-worker` (ADR 0016 section 3): each
#     pod serves HTTP and runs the Postgres queue worker in one process.
#     CronJobs keep the entrypoint and override only the arguments, for
#     example `args: ["task", "<name>"]` or `args: ["jobs", "tidy"]`.
#   - SOURCE_COMMIT is the full commit SHA, passed by .tekton/push.yaml as
#     SOURCE_COMMIT={{ revision }}. The admin build inlines it as
#     VITE_SOURCE_COMMIT, so the footer links to the running source
#     (AGPL-3.0 section 13, ADR 0012 section 4). An empty value links to
#     /tree/master.

ARG NODE_VERSION=24
ARG RUST_VERSION=1.99.1
ARG SOURCE_COMMIT=""

########################################################################
# Stage admin: npm ci and npm run build in admin/ -> admin/dist
########################################################################
FROM registry.access.redhat.com/ubi9/nodejs-${NODE_VERSION}@sha256:ecdcc047222dc57edbdd07879a54b97d7ff9232d740d608e188b41315b5462de AS admin

ARG NODE_VERSION
ARG SOURCE_COMMIT

# The Node.js images run as UID 1001 (group 0) with HOME=/opt/app-root/src,
# which that user owns. The admin directory is created by RUN, as UID 1001,
# rather than by WORKDIR, which would create it owned by root and leave npm
# unable to write node_modules and dist.
WORKDIR /opt/app-root/src

# Fail early if the pinned digest does not carry the Node.js major the build
# expects (for example after a careless digest bump), and reject a
# SOURCE_COMMIT that is not a full commit SHA, since the footer would link to
# a page that does not exist.
RUN node_major="$(node -p 'process.versions.node.split(".")[0]')" \
 && if [ "$node_major" != "$NODE_VERSION" ]; then \
      echo "error: the admin base image has Node.js $node_major, but NODE_VERSION is $NODE_VERSION" >&2; exit 1; \
    fi \
 && case "$SOURCE_COMMIT" in \
      "") ;; \
      *[!0-9a-f]*) echo "error: SOURCE_COMMIT must be a full 40-character commit SHA or empty" >&2; exit 1 ;; \
      *) if [ "${#SOURCE_COMMIT}" -ne 40 ]; then echo "error: SOURCE_COMMIT must be a full 40-character commit SHA or empty" >&2; exit 1; fi ;; \
    esac \
 && mkdir admin

WORKDIR /opt/app-root/src/admin

# The lock file alone decides the dependency tree (npm ci). Copying it before
# the sources lets local podman builds reuse the install layer.
COPY --chown=1001:0 admin/package.json admin/package-lock.json ./
RUN npm ci --no-audit --no-fund

COPY --chown=1001:0 admin/ ./
# The ts-rs bindings in admin/src/bindings/ are committed, so this stage needs
# no Rust (ADR 0004 section 2). The chmod makes dist readable for any UID.
RUN VITE_SOURCE_COMMIT="$SOURCE_COMMIT" npm run build \
 && test -f dist/index.html \
 && chmod -R a+rX dist

########################################################################
# Stage rust: rustup + cargo build --release --locked -> backend3-cli
########################################################################
FROM registry.access.redhat.com/ubi9/ubi@sha256:217dd5692d369799d34d09ffb6f5f3404efebc7900834490fdf9ae92fd6721d4 AS rust

ARG RUST_VERSION
# rustup-init itself; bump both together from
# https://static.rust-lang.org/rustup/archive/<version>/x86_64-unknown-linux-gnu/rustup-init.sha256
ARG RUSTUP_VERSION=1.29.2
ARG RUSTUP_INIT_SHA256=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4

# RUSTUP_TOOLCHAIN overrides rust-toolchain.toml inside the build. The two are
# equal (the Toolchain CI job checks it); the override keeps rustup from
# downloading the toolchain file's rustfmt and clippy components, which a
# release build does not need.
ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH \
    RUSTUP_TOOLCHAIN=${RUST_VERSION} \
    CARGO_TERM_COLOR=never

RUN dnf install -y --nodocs --setopt=install_weak_deps=0 gcc \
 && dnf clean all \
 && rm -rf /var/cache/dnf

RUN dl="$(mktemp -d)" \
 && curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
      --output "$dl/rustup-init" \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
 && echo "${RUSTUP_INIT_SHA256}  $dl/rustup-init" | sha256sum --check --strict - \
 && chmod 0755 "$dl/rustup-init" \
 && "$dl/rustup-init" -y --no-modify-path --profile minimal --default-toolchain "${RUST_VERSION}" \
 && rm -rf "$dl" \
 && rustc --version \
 && cargo --version

WORKDIR /src
COPY . .
# The same SOURCE_COMMIT reaches rustc, where App::app_version reads it with
# option_env!, so `backend3-cli version` and the start banner name the
# running commit. A malformed value is rejected by the admin stage, which
# fails the whole build.
# Declared here, after COPY, so it invalidates no earlier layer.
ARG SOURCE_COMMIT
RUN SOURCE_COMMIT="$SOURCE_COMMIT" cargo build --release --locked \
 && test -x target/release/backend3-cli

########################################################################
# Stage runtime: ubi-micro with the binary, config/ and admin/dist
########################################################################
FROM registry.access.redhat.com/ubi9/ubi-micro@sha256:5be877a2a280ae36eb9ab80ca4efb28bbf77c57abbba38137cdc37a03b8465e6 AS runtime

ARG SOURCE_COMMIT

# ubi-micro already has glibc and libgcc_s, which a Rust binary links, but no
# CA bundle. Copy the one from the UBI 9 build stage to the standard RHEL path
# and point SSL_CERT_FILE at it, so outbound TLS that uses the system roots
# works.
COPY --from=rust /etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem /etc/pki/tls/certs/ca-bundle.crt
ENV SSL_CERT_FILE=/etc/pki/tls/certs/ca-bundle.crt

WORKDIR /app
COPY --from=rust /src/target/release/backend3-cli /app/backend3-cli
COPY --from=rust /src/config/ /app/config/
# --from keeps the admin stage's owner (UID 1001); --chown makes dist root's,
# like every other file here, so the runtime user cannot modify it.
COPY --from=admin --chown=0:0 /opt/app-root/src/admin/dist/ /app/admin/dist/

# The Deployment and the CronJobs set LOCO_ENV too. The default keeps a bare
# `podman run` from falling back to Loco's `development` environment.
ENV LOCO_ENV=production

# Replace the inherited Red Hat labels that describe the base image, and add
# the OCI ones ghcr.io uses to link the package to its repository.
LABEL name="website_backend3" \
      summary="backend3: admin API, admin UI and public read API of the Verseghy website" \
      description="Loco (Rust) application serving /admin, /api/admin and /api/v1" \
      io.k8s.display-name="backend3" \
      io.k8s.description="Loco (Rust) application serving /admin, /api/admin and /api/v1" \
      vendor="Verseghy" \
      maintainer="Verseghy" \
      url="https://github.com/Verseghy/website_backend3" \
      vcs-type="git" \
      vcs-ref="${SOURCE_COMMIT}" \
      org.opencontainers.image.source="https://github.com/Verseghy/website_backend3" \
      org.opencontainers.image.revision="${SOURCE_COMMIT}" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.title="backend3"

# A numeric non-root user; OpenShift replaces it with the namespace's
# arbitrary UID (group 0) under restricted-v2.
USER 1001
EXPOSE 5150
ENTRYPOINT ["/app/backend3-cli"]
CMD ["start", "--server-and-worker"]
