import "react";

declare module "react" {
  // React's generic parameter is required for declaration merging but is not used by this augmentation.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface InputHTMLAttributes<T> {
    webkitdirectory?: boolean | string;
  }
}
