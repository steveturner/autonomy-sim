import { defineConfig, loadEnv } from 'vite';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '');
  const host = env.VITE_BIND_HOST || '127.0.0.1';

  return {
    server: { host },
    preview: { host },
  };
});
