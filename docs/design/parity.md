# Parity checklist

This file lists every feature backend3 must account for: each backend1 admin feature, each backend2 read the public site depends on, the release-1 additions and the platform work. Every row has a stable ID. ADR [0019] uses this file as the source for one GitHub issue per ID on the "website_backend3 parity" project board. An ID is done when the PR that closes its issue is merged; ADR [0019] §1 gives the closing rule for each kind of ID.

The board tracks status; this file does not. Changing a row's disposition is a PR to this file, and to the deciding ADR if the decision itself changes.

## How to read this file

| Column | Meaning |
|---|---|
| ID | Stable identifier. IDs are never renumbered or reused. New IDs are appended to their section. |
| Feature | What exists today (or, in the "new" sections, what is added), in one line |
| Disposition | `port`, `port-with-fix`, `replaced`, `retired` or `new` (below) |
| ADR | The ADR that decides the disposition. `api` is the public API contract, [public-api.md][api]. |
| Notes | The behaviour backend3 must have. It describes the target, not today's defects. |

| Disposition | Meaning |
|---|---|
| `port` | backend3 keeps the behaviour as it is today |
| `port-with-fix` | backend3 keeps the feature and changes the behaviour named in the notes |
| `replaced` | a different mechanism does the job; the notes name it |
| `retired` | not carried over; a documented exception in the ADR the row names, and listed in ADR [0019] §3 |
| `new` | exists in neither backend1 nor backend2 (release-1 additions, new capabilities, platform work) |

Two design-wide rules apply to every row and are not repeated per row: every admin operation is authorized as ADR [0007] defines, and every list cell is rendered as escaped text (ADR [0004]).

**Where the IDs come from.** These IDs are taken from the research unchanged: SHR, POS-H, POS-C (research: SYNTHESIS §2) and AUTH, RBAC, NAV, PAGE, MENU, FILE, EDIT, MEDIA, LOG, BKP, SITE, CLI, I18N (research: SYNTHESIS §3.1). This checklist mints the rest, because the research lists those behaviours in tables without IDs: POS-L, POS-F, LBL, AUT, COL, DSH, CAN, EVT, NWS (from SYNTHESIS §2.4-§2.11), PUB (from SYNTHESIS §5.16), NEW and PLT. The 32 "features that do not exist" in SYNTHESIS §3.14 (N1-N32) get no ID.

## 1. Shared behaviour of the content screens (SHR)

Source: research: SYNTHESIS §2.2.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| SHR-01 | Admin access for signed-in accounts | port-with-fix | [0006], [0007] | Every admin request needs an active, signed-in account (ADR 0006). |
| SHR-02 | Server-side paged list table, 25 rows by default | replaced | [0004], [0005] | ra-core list with server-side paging. Default 25 rows; page sizes 10, 25, 50 and 100; no "All" option. |
| SHR-03 | Default sort per entity, then id descending; sortable column headers | port | [0005] | Header sorting on stored columns |
| SHR-04 | One global search box over the entity's text columns | port-with-fix | [0020] | Case- and accent-insensitive substring match on the same columns. `%` and `_` are literal characters. |
| SHR-05 | List cells truncated to 40 characters | port-with-fix | [0004] | Long values are truncated in the list and shown in full on the edit page. |
| SHR-06 | Forms with save options (save and back, save and edit, save and new); tabs on posts | replaced | [0004], [0021] | The kit's form layout and save toolbar. Posts keep their tabs. Hungarian labels. |
| SHR-07 | Required-field marker | port-with-fix | [0004] | Shown on every field the server requires, including the post index image |
| SHR-08 | No hints, placeholders or explicit defaults | port-with-fix | [0021], [0018] | Hungarian help text where it helps (for example one award per line). Explicit defaults, such as today's date for a new post. |
| SHR-09 | Inputs trimmed; empty text saved as NULL | port | [0005] | Applied at the API boundary to optional text fields |
| SHR-10 | Only declared fields are saved | port | [0005] | Typed request DTOs: only declared, writable fields are written |
| SHR-11 | Many-to-many relations saved together with the row | port-with-fix | [0002], [0009] | The row and its relation changes are written in one transaction |
| SHR-12 | Delete after a confirmation dialog | replaced | [0011] | Delete moves the row to the trash. Restore and permanent delete happen on the trash screen. |
| SHR-13 | Checks on saved values happen in the database | replaced | [0009] | API validation matches every column limit, so the editor sees a form error, never a server error |
| SHR-14 | Relations kept consistent by application code | replaced | [0009] | Foreign keys with explicit delete rules |
| SHR-15 | Single-file upload field: show, replace, clear | port-with-fix | [0015], [0011] | Same directories and the same stored-value shape. A replaced or cleared file is removed only when no revision and no trashed row refers to it. |
| SHR-16 | Multi-file upload field (the post gallery) | port-with-fix | [0015], [0017] | Stored as an ordered array, with per-file remove. Legacy object-shaped values are converted at import. |
| SHR-17 | Upload size and count limits | replaced | [0015] | One upload size limit of about 128 MiB per file, and a MIME allow-list |
| SHR-18 | English admin with Hungarian validation messages | replaced | [0021] | Hungarian only (see I18N-01) |

## 2. Posts

### 2.1 Publishing workflow (POS-H)

Source: research: SYNTHESIS §2.4.5, §2.4.8.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| POS-H01 | Only holders of `publish posts` publish or unpublish; writers cannot change a published post | port-with-fix | [0018], [0007] | The writer lock is kept. Delete moves the post to the trash. |
| POS-H02 | Public visibility: published posts only | port | [0005], [api] | Published and not in the trash |
| POS-H03 | Public order by date, then id; archive buckets by the year and month of the date | port-with-fix | [0018], [api] | Date required (default today). Keyset paging returns every published post exactly once (backend2 fix #251). A nullable `publish_at` column is added and not used. |
| POS-H04 | Featured flag | port-with-fix | [0018] | Setting it requires `publish posts`. No count cap (as today). |
| POS-H05 | Preview token of 80 alphanumeric characters | port | [0017], [0018] | Legacy tokens are imported unchanged |
| POS-H06 | Preview token created when the edit form first opens | port-with-fix | [0018] | Created together with the draft and can be regenerated. Opening a form writes nothing. |
| POS-H07 | Preview link or public link on the Publish tab | port-with-fix | [0018] | The public host comes from configuration. A draft links to its preview URL, a published post to its public URL. |
| POS-H08 | Preview lookup: a token matches an unpublished post only | port | [api] | Exact match |
| POS-H09 | The token alphabet must survive query-string decoding | port | [0018] | New tokens use only `[A-Za-z0-9]` |
| POS-H10 | Deleting a post removes its files and its relations | replaced | [0011] | The trash keeps files, author and labels. Permanent delete removes them. |
| POS-H11 | Author and label references on a post | port-with-fix | [0009] | Foreign keys and existence validation |
| POS-H12 | A published post may have no author | port-with-fix | [0017], [0018], [api] | Posts 55, 61 and 66 get authors from the signed resolution list, and publishing requires an author. An author cannot be moved to the trash while a live post has it (ADR 0011). Only a draft preview can show `author: null`. |
| POS-H13 | Index image required on create; it can be replaced, never cleared | port | [0018], [0015], [0017] | Post 70, the one published post without an index image, gets one that editors set in the current admin before the final dump. The importer refuses a published post without an index image, and the schema checks it (ADR 0009, ADR 0017, [data-migration.md](data-migration.md) D3). |
| POS-H14 | Post date optional; some drafts have NULL or `1970-01-01` | port-with-fix | [0017], [0018] | The sentinel dates become NULL. A date is required on the next save. |
| POS-H15 | Posts written before 2022 show a date one day early | port-with-fix | [0017] | +1 day on the 181 posts in the signed resolution list |
| POS-H16 | Editing only the labels leaves the post's timestamp unchanged | port-with-fix | [0011] | Every change, labels included, updates `updated_at` and `updated_by` and writes a revision |
| POS-H17 | Labels on a post are ordered by label id, descending | port | [api] | |
| POS-H18 | Some published posts have NULL content | port-with-fix | [api] | Served as [public-api.md][api] specifies, never as an error |

### 2.2 Stored content and the editor (POS-C)

Source: research: SYNTHESIS §2.4.6, §3.8.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| POS-C01 | CKEditor 4 on post content, with a server file browser and an embed button | replaced | [0013], [0015] | Extended TipTap, with the media-library picker. No third-party embed service. |
| POS-C02 | Toolbar and block formats | replaced | [0013] | As ADR 0013 defines them |
| POS-C03 | Alt text required for images | port | [0013] | Required in the editor |
| POS-C04 | Inserted files use root-relative `/storage/uploads/...` URLs | port-with-fix | [0015] | Same URL form and the same `/storage/uploads/posts/<post-id>/` convention. The picker is open to every role with `manage media`. |
| POS-C05 | Pasted images stored inline as base64 | replaced | [0013], [0017] | Images from outside sources are never pasted (decisions 15d, 15f): pictures are stripped from pasted Word, Excel and web content, and a pasted image file is not inserted; the editor points to the image upload or the media library. Images copied or cut inside the editor, already under `/storage`, paste normally, and a file dropped from disk is uploaded like one chosen with the image button. No placeholder, no RTF extraction. Legacy inline images are extracted at import. |
| POS-C06 | Accented letters stored as HTML named entities | replaced | [0017], [0020] | Decoded at import. The editor writes UTF-8. Search uses a decoded plain-text column. |
| POS-C07 | Legacy markup from source mode and Word paste (bare iframes, `<center>`, Word styles) | replaced | [0013], [0017] | Normalised at import, with the original kept as a revision. After that, only sanitised HTML is stored. Old and new content share one content-v1 shape: tables stop stretching to 1000 px, floated images are centred, legacy YouTube sizes become a responsive player of at most 640 px, and images wider than 700 px are untouched (decision 15b). |
| POS-C08 | YouTube and padlet embeds render on the public site | port-with-fix | [0013], [0017] | Converted to editor nodes. Iframes are allowed only from YouTube and padlet embed URLs. |
| POS-C09 | Pasted spreadsheet tables with per-cell styles | port | [0013] | The allowed cell styles survive a round trip. Paste from Windows desktop Word and Excel into Chrome, Edge or Firefox keeps text colour, highlight, font size (stored relative to the body size) and the cell layout (borders, fills, widths, alignment). Font family is dropped (decisions 15c, 15e). Other Office setups are not targeted in v1. |
| POS-C10 | Server-side handling of submitted HTML | replaced | [0013] | The ammonia allow-list runs on every write. No raw HTML view. |
| POS-C11 | Bodies have been HTML since the 2020 Markdown conversion | port | [0017] | No Markdown handling is needed |

### 2.3 List screen and form (POS-L, POS-F)

Source: research: SYNTHESIS §2.4.2-§2.4.4, §2.4.7.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| POS-L01 | List columns: title, colour, index image, author, featured; search by title and author name | port-with-fix | [0004], [0020] | Thumbnails load from `/storage`. Date and published status are shown as well. |
| POS-L02 | "Draft" filter | port | [0004], [0021] | Hungarian label |
| POS-L03 | Default order: date descending, then id descending | port | [0018] | |
| POS-F01 | Form: title, description, date, labels, author, colour; tabs Properties, Content, Images, Publish | port-with-fix | [0018], [0009] | Length limits match the columns. Date required, with today as the default. The label picker works on the merged label list. |
| POS-F02 | Images tab: index image and gallery | port-with-fix | [0015] | The index image and the gallery accept images only (JPEG, PNG, GIF, WebP), as ADR 0015 §6 sets for entity image fields (research: DECISIONS 26b). Neither has a width, height or size rule beyond the global upload limit. Legacy non-image gallery entries stay and are still served. Images are optimised on upload. The gallery keeps upload order. |

## 3. Labels (LBL)

Source: research: SYNTHESIS §2.5.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| LBL-01 | Labels list, create, edit, delete; list shows name and colour; search by name; ordered by name | port-with-fix | [0007], [0011] | Delete moves the label to the trash. |
| LBL-02 | Fields: name (required, up to 64 characters) and colour (required, `#rrggbb`, colour picker) | port | [0009] | |
| LBL-03 | Label names are not unique | port-with-fix | [0017] | The duplicate groups are merged by the signed list, then names are unique |
| LBL-04 | Deleting a label detaches it from its posts | replaced | [0011] | Trashing keeps the links. Permanent delete removes them. |
| LBL-05 | Public: label chip colour and the label's post list | port-with-fix | [0005], [api] | Paging and errors as [public-api.md][api] specifies |

## 4. Authors (AUT)

Source: research: SYNTHESIS §2.6. Authors are by-lines on posts, not admin accounts.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| AUT-01 | Authors list, create, edit, delete; list shows name, description and image; search by name and description; ordered by name | port-with-fix | [0007], [0011] | Delete moves the author to the trash. |
| AUT-02 | Fields: name (required), short description, profile image | port-with-fix | [0009], [0015] | Length limits match the columns. The image accepts JPEG, PNG, GIF or WebP, with no width, height or size rule beyond the global upload limit. |
| AUT-03 | Author image stored under `authors_images/` | port-with-fix | [0015] | Same directory and stored-value shape. Images are optimised on upload. |
| AUT-04 | Deleting an author clears the author of its posts | replaced | [0011], [api] | Trashing is refused while a live post has the author. Permanent delete is refused while any post refers to it (ADR 0011). |
| AUT-05 | Public: author page and the post by-line | port-with-fix | [0005], [api] | A missing author answers 404. Paging as [public-api.md][api] specifies. |

## 5. Colleagues (COL)

Source: research: SYNTHESIS §2.7.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| COL-01 | Colleagues list, create, edit, delete; list shows name, jobs, image, category; search by name and jobs; ordered by name | port-with-fix | [0007], [0011], [0020] | Hungarian ordering. |
| COL-02 | Fields: name; jobs, subjects, roles, awards (text, up to 2,048 characters each); category (5 fixed values) | port-with-fix | [0009], [0021] | Category is required. The option labels match the public headings: "Vezetőség", "Tanárok", "Óraadók", "Nevelő, oktató munkát közvetlenül segítők", "Karbantartók, portások, takarítók". |
| COL-03 | Colleague image stored under `colleagues_images/` | port-with-fix | [0015] | Same directory and stored-value shape. JPEG, PNG, GIF or WebP; no size rule beyond the global upload limit. Images are optimised on upload. |
| COL-04 | Awards entered one per line (the public page splits them on line breaks) | port | [0021] | Shown as help text on the field |
| COL-05 | Deleting a colleague deletes the image | replaced | [0011] | Trash, then permanent delete |
| COL-06 | Public: colleagues by category, ordered by name ignoring a leading "Dr. ", with Hungarian collation | port | [0020] | |

## 6. Canteen dishes (DSH)

Source: research: SYNTHESIS §2.8. The old admin calls these "Menus".

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| DSH-01 | Dishes list, create, edit, delete; ordered by name | port-with-fix | [0007], [0011] | Delete moves the dish to the trash. |
| DSH-02 | Fields: name (required, up to 64 characters); course: soup, main dish, dessert | port | [0009], [0021] | Hungarian labels |
| DSH-03 | Dish names are not unique | port-with-fix | [0018] | Duplicates are kept. The pickers show how often each one is used, so they can be told apart. |
| DSH-04 | Deleting a dish removes it from the days that use it | replaced | [0011] | Trashing is refused while a live canteen day uses the dish (ADR 0011). |

## 7. Canteen days (CAN)

Source: research: SYNTHESIS §2.9. The old admin calls these "Canteens".

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| CAN-01 | Canteen days list, create, edit, delete; list shows the date and the dishes; ordered by date, descending | port-with-fix | [0007], [0011], [0018] | The list groups dishes by course. |
| CAN-02 | Day form: date (required) and 2-3 dishes | port-with-fix | [0018], [0017] | The single-day form is kept. Dates are unique, and the 2-3 dish rule applies on write. The 8 duplicate dates are resolved at import. |
| CAN-03 | Dish picker lists every dish by name | replaced | [0018] | Type-ahead pickers grouped by course, with inline dish creation and usage counts. See also the week editor, NEW-11. |
| CAN-04 | Deleting a day removes its dish links | replaced | [0011] | Trash, then permanent delete |
| CAN-05 | Public: the canteen menu of an ISO week, dishes ordered by course | port-with-fix | [0005], [api] | A nonexistent week answers 400 (backend2 fix #258) |

## 8. Events (EVT)

Source: research: SYNTHESIS §2.10.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| EVT-01 | Events list, create, edit, delete; list shows title, description, start and end; search; ordered by start, descending | port-with-fix | [0007], [0011], [0020] | Delete moves the event to the trash. |
| EVT-02 | Fields: title, description, start, end (end not before start), colour | port-with-fix | [0009] | Length limits match the columns. Colour required, as `#rrggbb`. |
| EVT-03 | Start and end are Budapest wall-clock times; whole-day events are entered as 00:00-23:59 | port-with-fix | [0010] | Stored as `timestamptz` (converted from Europe/Budapest at import). The API renders them back as naive Budapest time. |
| EVT-04 | Deleting an event | replaced | [0011] | Trash, then permanent delete |
| EVT-05 | Public: events that overlap a calendar month | port-with-fix | [0005], [api] | Covers the full Monday-Sunday grid (backend2 fix #248) |

## 9. Newsletter (NWS)

Source: research: SYNTHESIS §2.11.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| NWS-01 | Newsletter subscriber list (e-mail and token), reachable by URL only | retired | [0019] | The table is not migrated (ADR 0017). The newsletter is removed from the safety-net list (ADR 0011), and `edit newsletter` is not created (ADR 0007; open item in section 23). |
| NWS-02 | backend1's public subscribe and unsubscribe endpoints, which no frontend ever called | retired | [0019] | Not ported |

## 10. Authentication (AUTH)

Source: research: SYNTHESIS §3.1, §3.2.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| AUTH-01 | E-mail and password login | port-with-fix | [0006] | A Loco JWT in an httpOnly, Secure, SameSite=Strict cookie, valid for about 2 h and refreshed on activity. Legacy bcrypt hashes are verified once at login and rehashed to Argon2id. Bearer tokens remain for scripts. |
| AUTH-02 | "Remember me" | retired | [0006] | `remember_token` is not migrated. The sliding session takes its place. |
| AUTH-03 | Login throttling | port-with-fix | [0023], [0006] | Per-IP and per-account throttling with lockout and backoff, plus router-level limits |
| AUTH-04 | Logout | port-with-fix | [0006] | Logout revokes every token of the account (`token_version`) |
| AUTH-05 | Forgot-password flow by e-mail | replaced | [0006] | Admin-issued one-time reset links or temporary passwords, which must be changed at the first login. No e-mail. |
| AUTH-06 | Self-registration (closed) | retired | [0006] | Only admins create accounts |
| AUTH-07 | My account: name, e-mail, password change | port-with-fix | [0006] | The current password is required, and new passwords have at least 12 characters. TOTP enrolment as well (NEW-09). |
| AUTH-08 | Avatar in the user menu | port-with-fix | [0006] | Where the avatar comes from is an open item (below) |
| AUTH-09 | Account creation from the command line | port-with-fix | [0006] | Ported as a Loco task that creates an account with a role and a temporary password, which must be changed at the first login. It creates the first admin on a fresh development or staging database. In production the first accounts come only from the import (ADR 0003), which refuses a non-empty `users` table ([data-migration.md](data-migration.md) §4.2). |

## 11. Users, roles and permissions (RBAC)

Source: research: SYNTHESIS §3.1, §3.3.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| RBAC-01 | 4 roles and 9 named permissions, created by migrations | port-with-fix | [0007] | The same roles and 8 of the 9 names, plus `edit pages`, `edit menu`, `manage media` and `view logs`, and the four release-1 permissions: 16 in all. `edit newsletter` and `manage backups` are not created (open item in section 23). Roles are defined in code and migrations. |
| RBAC-02 | Users: list (name, e-mail, roles, extra permissions), filters by role and permission, create, edit with role and extra-permission checklists, delete | port-with-fix | [0007], [0006], [0011] | Per-user extra grants are kept. Permissions as ADR 0007 defines them. Account lifecycle in NEW-10. Users follow the safety net. |
| RBAC-03 | Roles list, read-only, with user counts | port | [0007] | |
| RBAC-04 | Permissions list, read-only | port | [0007] | |
| RBAC-05 | Permission checks and permission-gated navigation | replaced | [0007] | ADR 0007's permission model. The navigation hides what the user cannot use. |

## 12. Navigation and dashboard (NAV)

Source: research: SYNTHESIS §3.4.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| NAV-01 | Sidebar with permission-gated groups | port-with-fix | [0007], [0021], [0004] | Hungarian labels, gated by the same permissions. New entries for the media library, trash, history, activity log and the release-1 screens. No newsletter entry. |
| NAV-02 | Dashboard with a static image-size advice card ("Képek") | replaced | [0019] | Usage statistics (NEW-12, NEW-13). The card is not ported, because uploads are optimised automatically (ADR 0015). |

## 13. Pages (PAGE)

Source: research: SYNTHESIS §3.5.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| PAGE-01 | Pages list, create, edit, delete, clone; "Open" button | port-with-fix | [0018], [0007], [0011] | Clone generates a unique slug. "Open" goes to the public URL `/information/<slug>`. Trash and revisions. |
| PAGE-02 | Page fields: template, internal name, title, slug, meta fields, content | port-with-fix | [0018] | One page type: title, slug, internal note (formerly the name), content, an optional SEO description and a published/hidden flag. The API returns `template` as `page` and empty `extras`. The content column is widened. |

## 14. Menu (MENU)

Source: research: SYNTHESIS §3.6.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| MENU-01 | Menu items list, create, edit, delete | port-with-fix | [0018], [0007], [0011] | Parent cycles are rejected (ADR 0018). Trash and revisions. |
| MENU-02 | Item types: page link, internal link, external link | port-with-fix | [0018], [0009] | All three types are kept. A non-page item stores no page reference; the foreign key makes the importer drop stray references. |
| MENU-03 | Drag-and-drop reorder screen over a nested set | replaced | [0018] | A clean-room open-source shadcn tree component with drag and drop, stored as `parent_id` plus `position`. The menu is capped at 2 levels. The import keeps today's served order. |

## 15. Files, editor and media (FILE, EDIT, MEDIA)

Source: research: SYNTHESIS §3.7, §3.8.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| FILE-01 | File manager over the `uploads/` folder | replaced | [0015] | A minimal media library: browse folders, upload (button, drag and drop into the library; an image pasted into the editor is never uploaded, decision 15d), rename and move, delete, copy URL, insert into the editor. It offers only these operations. A MIME allow-list (images, PDF, Office documents), about 128 MiB per file, `manage media` for the content roles, paths unchanged. |
| FILE-02 | "Browse server" from the editor's image and link dialogs | replaced | [0013], [0015] | The media-library picker inside the editor |
| EDIT-01 | CKEditor 4 on post and page content, with alt text required | replaced | [0013] | Extended TipTap. Paste from Word and Excel as POS-C09 defines. The editor surface matches website_frontend2's PageRenderer styling (decision 15a), and content is stored in the content-v1 HTML shape. Alt text required. The ammonia allow-list runs on every write. No raw HTML view. |
| EDIT-02 | Media embed button through a third-party oEmbed service | replaced | [0013], [0017] | A YouTube node (which also parses bare iframes) and a padlet node. Legacy embed wrappers are converted at import. |
| MEDIA-01 | Upload fields per entity: generated file names; replace, clear, delete with the row | port-with-fix | [0015], [0011] | Same directories, same stored-value shape and same URLs. Files are removed only on permanent delete, and only when nothing refers to them. Images are optimised on upload. |
| MEDIA-02 | Public `/storage/**` on both hosts, with `Access-Control-Allow-Origin: *` | port-with-fix | [0015] | A separate unprivileged `ubi9/nginx-124` serves `/storage` on both hosts with `Access-Control-Allow-Origin: *`, real 404s, no directory listing and range requests. backend3 co-mounts the volume for uploads. If the co-mount test fails, backend3 serves `/storage` itself with a dedicated ServeDir router, at 1 replica with `Recreate`. |
| MEDIA-03 | Draft preview link with a token on the post form | port-with-fix | [0018] | See POS-H06 and POS-H07 |

## 16. Operations screens and commands (LOG, BKP, SITE, CLI)

Source: research: SYNTHESIS §3.9-§3.12.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| LOG-01 | Logs screen over the application's log files | retired | [0019] | Operators use `oc logs` and the runbook. Who changed what is in the activity log (NEW-07). |
| BKP-01 | Backups screen | retired | [0019] | Backups run outside the application (ADR 0023, private runbook). backend3 has no backup screen. |
| SITE-01 | Sitemap of published posts and pages | port-with-fix | [0019] | Generated by backend3 from published posts and visible pages; hidden and trashed items are left out. `lastmod` is the item's last change. The URL `/storage/sitemap.xml` is unchanged. |
| CLI-01 | Daily scheduled sitemap command | replaced | [0016], [0019] | Scheduled work runs as Kubernetes CronJobs that run backend3's Loco tasks with the app binary (ADR 0016). The sitemap is also regenerated after each relevant content change. |

## 17. Localisation (I18N)

Source: research: SYNTHESIS §3.13.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| I18N-01 | English admin UI with Hungarian validation messages and help text | replaced | [0021] | Hungarian only: one complete Hungarian message file in the repository (react-admin keys, kit and rich-text strings, every label). English is used only as the fallback for a missing key. |

## 18. Public read API (PUB)

These rows track the backend2 reads the Angular site depends on. They are needed to retire backend2 (ADR [0019]). Each endpoint's paths, wire shapes, errors and fixes are in [public-api.md][api]. Its tests are written from that contract and must cover the production-only data shapes listed in ADR [0019]. Here `port-with-fix` means the same data and meaning over REST, with the fixes the contract names.

Source: research: SYNTHESIS §5.4, §5.16.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| PUB-01 | Home-page post list with a featured filter and keyset paging | port-with-fix | [0005], [api] | [public-api.md][api] §7.1. Fixed cursor (backend2 fix #251), page info computed within the filter, a bounded page size. |
| PUB-02 | Post search | port-with-fix | [0020], [api] | §7.2. Searches the decoded plain-text column, case- and accent-insensitive. |
| PUB-03 | Single post, including the draft preview with a token | port-with-fix | [0018], [api] | §7.3. A missing post answers 404. |
| PUB-04 | Author and the author's posts | port-with-fix | [0005], [api] | §7.4 |
| PUB-05 | Label and the label's posts | port-with-fix | [0005], [api] | §7.5 |
| PUB-06 | Archive index: post counts per year and month | port | [0005], [api] | §7.6 |
| PUB-07 | Archive month: the posts of one month | port-with-fix | [0005], [api] | §7.7. Ties broken by id. |
| PUB-08 | Canteen week by ISO year and week | port-with-fix | [0005], [api] | §7.8, plus the current-weeks endpoint of §7.9. Correct across year boundaries (backend2 fix #258). |
| PUB-09 | Events of a calendar month | port-with-fix | [0010], [api] | §7.10. Full grid; naive Budapest times on the wire. |
| PUB-10 | Information menu tree | port-with-fix | [0018], [api] | §7.11. Two levels; today's order kept. |
| PUB-11 | Page by slug | port-with-fix | [0018], [api] | §7.12. Hidden and unknown pages answer 404. |
| PUB-12 | Colleagues | port | [0020], [api] | §7.13 |
| PUB-13 | API description and health endpoints | replaced | [0005] | §7.18. The GraphQL schema and GraphiQL are replaced by an OpenAPI 3.1 document at `/api/v1/openapi.json`, snapshot-tested, plus Loco's health endpoints. |

## 19. Release-1 additions (NEW-01 to NEW-04)

Each one gets an admin screen, an admin API and a public read endpoint. The Angular site switches to them with its REST migration. ADR [0019] gives the fields and the permissions.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| NEW-01 | Home-page notices: message, link, priority (`low`, `medium`, `high`), visible until | new | [0019] | [public-api.md][api] §6.11, §7.14. Replaces the array hard-coded in Angular's `alert.service.ts`. |
| NEW-02 | Partner banners: image, alt text, link, height, position, visible flag | new | [0019] | §6.12, §7.15. The three current banners are seeded at cut-over. |
| NEW-03 | Site settings: address, phone numbers (each with an optional label), the contact e-mail shown in the footer, the website-feedback e-mail on the footer's bottom line, OM identifier | new | [0019] | §6.13, §7.16. One record, seeded from the current Angular footer. |
| NEW-04 | Diáktoll issues: title, date, PDF, optional cover image, published flag | new | [0019] | §6.14, §7.17. The existing PDF is seeded as the first issue. |

## 20. Other new capabilities (NEW-05 to NEW-14)

These are capabilities that other decisions add. They do not map onto a single legacy row.

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| NEW-05 | Trash: soft delete, restore and permanent delete for every admin-editable entity | new | [0011] | One generic convention and one trash UI pattern. A clean-room build, never copying Enterprise source. |
| NEW-06 | Revisions: a JSON snapshot on every save, with view and restore | new | [0011] | One generic revisions table |
| NEW-07 | Activity log: append-only record of who did what and when, with an admin screen | new | [0011] | |
| NEW-08 | Audit columns `created_by` and `updated_by` (and `deleted_by` where the entity has a trash) on every admin-editable entity table that ADR 0011 §1 lists | new | [0011] | |
| NEW-09 | TOTP two-factor login with recovery codes | new | [0006] | Available to every account, mandatory for none |
| NEW-10 | Account lifecycle: active flag, bulk deactivate, last login shown | new | [0006] | Deactivated users keep their authorship and history |
| NEW-11 | Canteen week editor: ISO week, Monday-Friday rows, pickers grouped by course | new | [0018] | Inline dish creation; duplicate names shown with usage counts |
| NEW-12 | Dashboard usage statistics: posts per month, author and label; drafts waiting; edits per user; canteen coverage; media storage | new | [0019] | Each widget is gated by a permission |
| NEW-13 | Self-hosted read counter for public post and page reads | new | [0019] | Daily counts only. No cookies, IP addresses or user agents. |
| NEW-14 | Automatic image optimisation on upload | new | [0015] | Visually lossless re-encode (about q85) and metadata strip. Format, dimensions, URL and extension stay the same. |

## 21. Platform items (PLT)

Repository, delivery, deployment and cut-over work. Items in website_k8s and website_frontend are opened as PRs there and left for human review (ADR [0012]). Their PR bodies carry `Closes Verseghy/website_backend3#N` (ADR [0019] §1).

| ID | Feature | Disposition | ADR | Notes |
|---|---|---|---|---|
| PLT-01 | Repository settings: `OWNERS`, the Prow and PAC GitHub Apps installed, `lgtm` and `approved` labels present, no ruleset that requires reviews | new | [0012] | |
| PLT-02 | Tide query that needs only `approved`, for this repository | new | [0012] | website_k8s PR #50, merged by the maintainer on 2026-10-24 |
| PLT-03 | website_backend3's required CI contexts in Prow branch protection | new | [0022], [0012] | A follow-up website_k8s PR once the jobs exist |
| PLT-04 | `PR title` and `No session links` checks | new | [0012] | Both are required contexts and have their own tests |
| PLT-05 | `THIRD_PARTY.md`, licence files next to vendored code, cargo-deny and npm licence checks | new | [0012] | |
| PLT-06 | `AGENTS.md` and `CLAUDE.md` at the root and in `admin/` | new | [0024] | |
| PLT-07 | Vendored Loco skill, `scripts/sync-loco-skill.sh` and the `Loco skill` CI check | new | [0024] | |
| PLT-08 | `.claude/settings.json` deny rules and the `PreToolUse` hook | new | [0024] | The hook has its own tests |
| PLT-09 | Test harness and CLI refuse non-local databases (the `Hooks::load_config` guard) | new | [0024] | |
| PLT-10 | Tracked `CLAUDE.md` and guardrails in website_k8s | new | [0024] | A website_k8s PR |
| PLT-11 | Sanitised specs in `docs/design/`; the full research in the private runbooks repository | new | [0024] | |
| PLT-12 | One-day spike: utoipa with Loco | new | [0005] | Before the first API slice |
| PLT-13 | GitHub Actions PR checks: fmt, clippy, tests against Postgres, ts-rs and OpenAPI drift, admin lint, typecheck and unit tests, E2E | new | [0022] | |
| PLT-14 | PAC `Repository` CR (merged before the bootstrap PR merges) and `.tekton/push.yaml` with explicit nested timeouts | new | [0022] | Builds on `master` only; production images are never built by GitHub Actions |
| PLT-15 | More capacity for the `buildah-ubi9` build step (about 4 CPU and 6Gi) | new | [0022] | A website_k8s PR |
| PLT-16 | `Containerfile`: UBI 9 multi-stage, Rust 1.94 or later, a Node stage for `admin/` | new | [0004], [0022] | |
| PLT-17 | PostgreSQL Deployment, PVC, Service and Argo CD Application | new | [0002] | |
| PLT-18 | backend3 Deployment, Service, Route on `backend3.microshift.verseghy-gimnazium.net`, and an Application with the first image tag set by hand | new | [0014], [0022] | |
| PLT-19 | Image Updater entry for backend3, and a check that an automatic bump actually happens | new | [0022] | |
| PLT-20 | backend3's keys in the shared `backend-secrets` ExternalSecret | new | [0023] | |
| PLT-21 | `/storage` media server (`ubi9/nginx-124`) and the co-mount test on a throwaway PVC | new | [0015] | |
| PLT-22 | CronJobs for scheduled tasks | new | [0016] | |
| PLT-23 | HAProxy router connection and request limits | new | [0023] | On every backend3 Route from its first deploy (day one); annotations checked against the MicroShift docs for the live version |
| PLT-24 | `/metrics` exposed; monitoring planned on the hub only | new | [0023] | No scraping or monitoring stack on the edge node; monitoring planned on the hub with the OpenShift Monitoring stack |
| PLT-25 | The new database and volumes added to the backup procedure | new | [0023] | The backup system is otherwise kept as it is (decision 32a); private runbook |
| PLT-26 | Staging on the hub (LAN-only) for cut-over rehearsals | new | [0003] | |
| PLT-27 | Nightly import rehearsals against dumps | new | [0003], [0017] | |
| PLT-28 | Cut-over runbook: evening admin write freeze, final import, promotion of the Angular REST build | new | [0003] | |
| PLT-29 | backend2 fixes #251 and #258 merged now (decision 2), by the maintainer | retired | [0026] | Not done: the rewrite makes no change in website_backend2. backend3's `/api/v1` carries both fixes (PUB-01, PUB-08). |
| PLT-30 | Angular REST client build staged ahead, including notices, banners, settings and Diáktoll | new | [0003], [0019] | website_frontend PR |
| PLT-31 | Read-only parallel run of backend1 and MySQL as a reference view, with MySQL in `super_read_only`; PVs patched to `Retain` before any other cut-over change; decommissioned only when the maintainer decides, never on the project's initiative | new | [0003] | Closes once the parallel run is in place (ADR 0019 §1) |
| PLT-32 | Credential rotation at the cut-over | new | [0023] | |
| PLT-33 | backend2 and its Redis retired together with backend1 and MySQL when the maintainer decides to decommission; the website_frontend2 hub beta scaled down then | new | [0019], [0026] | Not in the window: backend2 and its Redis keep running, unchanged, through the read-only parallel run. Closes with the website_k8s PR that removes their manifests at decommissioning ([cutover.md](cutover.md) §10), never on the project's initiative |
| PLT-34 | Clean-up of files the retired backup tool used (procedure in the private runbook) | new | [0019] | |
| PLT-35 | Backstage `catalog-info.yaml` for website-backend3 | new | none | House convention (research: SYNTHESIS §1.16.3) |
| PLT-36 | Human review of the whole codebase after all slices have merged | new | [0012] | Outside the per-PR loop |
| PLT-37 | Every backup and rehearsal dump of production MySQL is taken with `mysqldump --lock-all-tables`; production MySQL tables stay untouched until the cut-over; backend2 migration 12 is never applied | new | [0003], [0017] | Backup script and rehearsal tooling (private runbook); checked by [cutover.md](cutover.md) P7 |
| PLT-38 | Admin hardening in code: `remote_ip` trusting only the router hop, `secure_headers` with the admin CSP, a capped page size on every public list endpoint | new | [0023] | Checked by [cutover.md](cutover.md) P12 |
| PLT-39 | Parity board and `scripts/sync-parity-issues.sh` with its tests | new | [0019], [0012] | Merges after the design PR and before the bootstrap PR opens. Its issue is created closed (ADR 0019 §1). |

## 22. Counts

| Section | IDs |
|---|---|
| Shared behaviour (SHR) | 18 |
| Posts (POS-H, POS-C, POS-L, POS-F) | 34 |
| Labels, authors, colleagues, dishes, canteen days, events, newsletter | 32 |
| Platform features of backend1 (AUTH, RBAC, NAV, PAGE, MENU, FILE, EDIT, MEDIA, LOG, BKP, SITE, CLI, I18N) | 33 |
| Public read API (PUB) | 13 |
| New (NEW) | 14 |
| Platform items (PLT) | 39 |
| **Total** | **183** |

## 23. Open items

These rows depend on something no decision has settled yet. Each is settled in the named place before its slice merges.

| Item | Rows | How it is settled |
|---|---|---|
| Where the user-menu avatar comes from: Gravatar (a third-party request that the admin CSP would have to allow) or initials, which need no request | AUTH-08 | The maintainer decides in the auth slice; ADR 0006 and ADR 0023 record it |
| `manage backups` and `edit newsletter`: decision 6 lists them, decisions 28 and 30 retire their screens, and the docs do not create them | RBAC-01, NWS-01 | The maintainer confirms in the RBAC slice; ADR 0007 records it |

[0002]: adr/0002-postgresql-as-the-database.md
[0003]: adr/0003-cut-over-one-rehearsed-flip-then-a-read-only-parallel-run.md
[0004]: adr/0004-admin-ui-lives-in-admin-and-ships-in-the-backend3-image.md
[0005]: adr/0005-rust-dtos-are-the-api-contract-ts-rs-utoipa-openapi.md
[0006]: adr/0006-admin-authentication-and-account-lifecycle.md
[0007]: adr/0007-authorization-four-roles-enforced-on-every-operation.md
[0009]: adr/0009-clean-domain-schema-with-preserved-integer-ids.md
[0010]: adr/0010-all-instants-stored-as-timestamptz.md
[0011]: adr/0011-safety-net-soft-delete-revisions-and-an-activity-log-for-eve.md
[0012]: adr/0012-delivery-review-commits-and-licensing.md
[0013]: adr/0013-rich-text-editor-and-html-sanitiser.md
[0014]: adr/0014-one-host-backend3-microshift-verseghy-gimnazium-net.md
[0015]: adr/0015-media-separate-static-server-media-library-image-optimisatio.md
[0016]: adr/0016-no-redis-in-memory-cache-postgres-queue-cronjob-tasks.md
[0017]: adr/0017-legacy-data-import-normalisation-and-defect-resolution.md
[0018]: adr/0018-content-model-publishing-pages-menu-tree-canteen-week-editor.md
[0019]: adr/0019-scope-parity-tracking-retired-features-release-1-additions.md
[0020]: adr/0020-search-and-hungarian-ordering.md
[0021]: adr/0021-hungarian-only-admin-ui.md
[0022]: adr/0022-builds-and-ci-pac-for-production-images-github-actions-for-p.md
[0023]: adr/0023-operations-hardening-rate-limits-monitoring-backups-secrets.md
[0024]: adr/0024-ai-maintainability-instruction-files-the-loco-skill-guardrai.md
[0026]: adr/0026-backend2-unchanged-and-running-until-decommissioning.md
[api]: public-api.md
