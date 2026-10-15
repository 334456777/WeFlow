/// <reference types="vitest/config" />
import { readFileSync } from 'node:fs';
import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv, type Plugin, type ProxyOptions } from 'vite';

const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf8'));

// What the build says about itself: compiled into the app, and written to dist/weflow-web.json for
// `weflow web` to check before it installs or picks this build (#148)
const meta = {
  name: pkg.name as string,
  version: pkg.version as string,
  apiVersion: pkg.weflow.apiVersion as string,
  minBackendVersion: pkg.weflow.minBackendVersion as string,
};

function webMeta(): Plugin {
  return {
    name: 'weflow-web-meta',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'weflow-web.json', source: JSON.stringify(meta, null, 2) + '\n' });
    },
  };
}

export default defineConfig(({ mode }) => {
  // WEFLOW_* from the shell or web/.env.local; `weflow serve` reads the same WEFLOW_HTTP_TOKEN
  const env = loadEnv(mode, process.cwd(), 'WEFLOW_');
  const target = env.WEFLOW_SERVE_URL || 'http://127.0.0.1:57108';
  const token = env.WEFLOW_HTTP_TOKEN;

  // Everything outside /ui/ belongs to `weflow serve`: the command routes (/chat/sessions), /api/, /jobs, /health,
  // /openapi.json. `/` itself stays with Vite, which sends it on to /ui/.
  const api: ProxyOptions = {
    target,
    changeOrigin: true,
    configure(proxy) {
      // The token stays on the developer's machine: added here, never compiled into the app
      if (!token) return;
      proxy.on('proxyReq', req => {
        if (!req.getHeader('authorization')) req.setHeader('authorization', 'Bearer ' + token);
      });
    },
  };
  const proxy = { '^/(?!ui(/|$)|$)': api };

  return {
    // weflow serve hosts the WebUI under /ui/ (#146)
    base: '/ui/',
    define: { __WEFLOW_WEB__: JSON.stringify(meta) },
    plugins: [react(), webMeta()],
    server: { proxy },
    preview: { proxy },
    test: { environment: 'jsdom' },
  };
});
