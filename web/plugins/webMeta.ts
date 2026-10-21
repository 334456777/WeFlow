// What the build says about itself: compiled into the app (`__WEFLOW_WEB__`), and written to dist/weflow-web.json for
// `weflow web` to check before it installs or picks this build (#148). The tests define the same (test/setup.ts).
import type { Plugin } from 'vite';
import pkg from '../package.json' with { type: 'json' };

export const WEB_META = {
  name: pkg.name,
  version: pkg.version,
  apiVersion: pkg.weflow.apiVersion,
  minBackendVersion: pkg.weflow.minBackendVersion,
};

export function webMeta(): Plugin {
  return {
    name: 'weflow-web-meta',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'weflow-web.json', source: JSON.stringify(WEB_META, null, 2) + '\n' });
    },
  };
}
