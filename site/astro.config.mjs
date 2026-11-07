import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://0funct0ry.github.io',
  base: '/fluxwire',
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'Fluxwire',
      logo: { src: './src/assets/logo.svg' },
      description: 'Documentation for Fluxwire, a native API client whose requests live in your repository.',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/0funct0ry/fluxwire' }],
      customCss: ['./src/styles/docs.css'],
      head: [
        { tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.googleapis.com' } },
        { tag: 'link', attrs: { rel: 'stylesheet', href: 'https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=IBM+Plex+Sans:wght@400;500;600;700&display=swap' } },
      ],
      sidebar: [
        { label: 'Getting started', items: [
          'docs/getting-started/install', 'docs/getting-started/quick-start', 'docs/getting-started/first-collection'] },
        { label: 'Guides', items: [
          'docs/guides/environments-and-secrets', 'docs/guides/scripting', 'docs/guides/folder-runs',
          'docs/guides/websockets', 'docs/guides/importing', 'docs/guides/openapi-sync',
          'docs/guides/writing-docs', 'docs/guides/running-in-ci', 'docs/guides/for-ai-agents'] },
        { label: 'Reference', items: [
          'docs/reference/flux-format', 'docs/reference/front-matter', 'docs/reference/extending-the-format',
          'docs/reference/fw-api', 'docs/reference/cli', 'docs/reference/configuration',
          'docs/reference/keybindings', 'docs/reference/roles', 'docs/reference/exit-codes'] },
        { label: 'Project', items: [
          'docs/project/contributing'] },
      ],
    }),
  ],
});
