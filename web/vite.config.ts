import react from '@vitejs/plugin-react';
import { defineConfig, loadEnv, type ProxyOptions } from 'vite';
import { iconNames } from './plugins/iconNames.ts';
import { iconWeights } from './plugins/iconWeights.ts';
import { plainImport } from './plugins/plainImport.ts';
import { viteReactAlias } from './plugins/reactAlias.ts';
import { BUDGET, sizeBudget } from './plugins/sizeBudget.ts';
import { WEB_META, webMeta } from './plugins/webMeta.ts';

export default defineConfig(({ mode }) => {
  // WEFLOW_* from the shell or web/.env.local; `weflow serve` reads the same WEFLOW_HTTP_TOKEN
  const env = loadEnv(mode, process.cwd(), 'WEFLOW_');
  const target = env.WEFLOW_SERVE_URL || 'http://127.0.0.1:18108';
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
    define: { __WEFLOW_WEB__: JSON.stringify(WEB_META) },
    resolve: { alias: viteReactAlias() },
    optimizeDeps: { exclude: ['react', 'react-dom'] },
    // jsxImportSource so the page's JSX is Preact's; the alias covers `react` and `react-dom` imports, including
    // the icon package, which calls React's createElement
    plugins: [react({ jsxImportSource: 'preact' }), iconWeights(), iconNames(), plainImport(), webMeta(), sizeBudget()],
    build: {
      // Vite's own warning at the same size as the budget (#159)
      chunkSizeWarningLimit: BUDGET.js / 1000,
      // The page is one chunk and the English one imports nothing, so no `<link rel="modulepreload">` is ever
      // written: the polyfill, which fetches those links where the browser does not, would only take up room (#783)
      modulePreload: { polyfill: false },
    },
    server: { proxy },
    preview: { proxy },
  };
});
