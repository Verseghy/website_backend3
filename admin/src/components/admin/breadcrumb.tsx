import * as React from "react";
import { createPortal } from "react-dom";
import { Separator } from "@/components/ui/separator";
import {
  Breadcrumb as BaseBreadcrumb,
  BreadcrumbEllipsis,
  BreadcrumbItem,
  BreadcrumbPage,
  BreadcrumbList,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";
import { buttonVariants } from "@/components/ui/button";
import {
  Drawer,
  DrawerClose,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
  DrawerTrigger,
} from "@/components/ui/drawer";
import { useIsMobile } from "@/hooks/use-mobile";
import { Translate, useTranslate } from "ra-core";
import { cn } from "@/lib/utils";

/**
 * A breadcrumb navigation component with mobile drawer support.
 *
 * Renders breadcrumb navigation in the app header via portal. On mobile, shows a drawer with
 * ellipsis for long breadcrumb trails. Use Breadcrumb.Item and Breadcrumb.PageItem as children.
 *
 * CRUD pages already include a Breadcrumb by default; set `disableBreadcrumb` to hide the
 * breadcrumb and/or provide your own.
 *
 * @see {@link https://marmelab.com/shadcn-admin-kit/docs/breadcrumb/ Breadcrumb documentation}
 * @see {@link https://ui.shadcn.com/docs/components/breadcrumb Breadcrumb UI documentation}
 *
 * @example
 * import { Edit, Breadcrumb, SimpleForm } from "@/components/admin";
 * import { LinkBase, RecordRepresentation } from 'ra-core';
 *
 * const PostEdit = () => (
 *   <Edit disableBreadcrumb>
 *     <Breadcrumb>
 *       <Breadcrumb.Item><LinkBase to="/">Home</LinkBase></Breadcrumb.Item>
 *       <Breadcrumb.Item><LinkBase to="/posts">Articles</LinkBase></Breadcrumb.Item>
 *       <Breadcrumb.PageItem>
 *         Edit Article "<RecordRepresentation />"
 *       </Breadcrumb.PageItem>
 *     </Breadcrumb>
 *     <SimpleForm>
 *       ...
 *     </SimpleForm>
 *   </Edit>
 * );
 */
export const Breadcrumb = ({ children, ref }: BreadcrumbProps) => {
  const breadcrumbPortal = document.getElementById("breadcrumb");
  const isMobile = useIsMobile();
  const translate = useTranslate();
  const [open, setOpen] = React.useState(false);
  if (!breadcrumbPortal) return null;
  return createPortal(
    <>
      <Separator
        orientation="vertical"
        className="data-[orientation=vertical]:h-4 mr-4"
      />
      <BaseBreadcrumb ref={ref}>
        <BreadcrumbList>
          {isMobile && React.Children.count(children) > 2 ? (
            <React.Fragment>
              <BreadcrumbItem>
                <Drawer open={open} onOpenChange={setOpen}>
                  <DrawerTrigger aria-label={translate("kit.breadcrumb.show_path")}>
                    <BreadcrumbEllipsis className="h-4 w-4" />
                  </DrawerTrigger>
                  <DrawerContent>
                    <DrawerHeader className="text-left">
                      <DrawerTitle>
                        <Translate i18nKey="ra.navigation.breadcrumb_drawer_title" />
                      </DrawerTitle>
                      <DrawerDescription>
                        <Translate i18nKey="ra.navigation.breadcrumb_drawer_instructions" />
                      </DrawerDescription>
                    </DrawerHeader>
                    <ol className="grid gap-1 px-4">
                      {React.Children.toArray(children)
                        .slice(0, -1)
                        .map((item) => item)}
                    </ol>
                    <DrawerFooter className="pt-4">
                      <DrawerClose
                        className={cn(buttonVariants({ variant: "outline" }))}
                      >
                        <Translate i18nKey="ra.action.close" />
                      </DrawerClose>
                    </DrawerFooter>
                  </DrawerContent>
                </Drawer>
              </BreadcrumbItem>
              <BreadcrumbSeparator />
              {React.Children.toArray(children).slice(-1)}
            </React.Fragment>
          ) : (
            React.Children.map(
              children,
              (child, index) =>
                child && (
                  <React.Fragment key={index}>
                    {child}
                    {index < React.Children.count(children) - 1 ? (
                      <BreadcrumbSeparator />
                    ) : null}
                  </React.Fragment>
                ),
            )
          )}
        </BreadcrumbList>
      </BaseBreadcrumb>
    </>,
    breadcrumbPortal,
  );
};
Breadcrumb.Item = BreadcrumbItem;
Breadcrumb.PageItem = BreadcrumbPage;

export { BreadcrumbItem, BreadcrumbPage };

export type BreadcrumbProps = React.ComponentProps<"nav">;
