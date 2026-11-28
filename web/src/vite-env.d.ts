/// <reference types="vite/client" />

/** 网页的名称、版本，以及它需要的 API 版本和最低的后端版本，取自 package.json（见 vite.config.ts）。 */
declare const __WEFLOW_WEB__: {
  name: string;
  version: string;
  apiVersion: string;
  minBackendVersion: string;
};
