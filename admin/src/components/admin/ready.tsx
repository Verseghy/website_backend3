import { ScanFace } from "lucide-react";
import { Translate } from "ra-core";

/**
 * Splash screen displayed when no resources are configured yet.
 *
 * Automatically shown when the admin app has no Resource children defined.
 *
 * backend3 change: Hungarian text from hu.ts, and no links to the kit's
 * documentation, demo or repository (see LOCAL_CHANGES.md).
 *
 * @see {@link https://marmelab.com/shadcn-admin-kit/docs/ready/ Ready documentation}
 */
export const Ready = () => (
  <div className="flex flex-col h-screen">
    <div
      className="flex-1 flex flex-col text-white text-center justify-center items-center"
      style={{
        background:
          "linear-gradient(135deg, #00023b 0%, #00023b 50%, #313264 100%)",
      }}
    >
      <ScanFace className="w-32 h-32 mb-4" />
      <h1 className="text-3xl mb-4">
        <Translate i18nKey="kit.ready.title" />
      </h1>
      <div className="text-lg opacity-75">
        <Translate i18nKey="kit.ready.message" />
      </div>
    </div>
  </div>
);
