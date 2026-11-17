// first: tokens.css states the order of the cascade layers, which holds only before any rule of a layer
import './styles/tokens.css';
// the default skin: the colours and the roles the pages use (#828)
import './styles/neumorphism.css';
import './app.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { adoptToken } from './api/client';
import { App } from './App';
import { languageReady } from './i18n';

adoptToken();

// in the language of the last visit from the start, not in Chinese first
void languageReady().then(() => createRoot(document.getElementById('root')!).render(
  <StrictMode><App /></StrictMode>,
));
