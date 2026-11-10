import React from 'react';
import { createRoot } from 'react-dom/client';
import { adoptToken } from './api/client';
import { App } from './App';
import { languageReady } from './i18n';
import { installSprings } from './motion';
import './styles/tokens.css';
import './app.css';

adoptToken();
installSprings();

// in the language of the last visit from the start, not in Chinese first
void languageReady().then(() => createRoot(document.getElementById('root')!).render(
  <React.StrictMode><App /></React.StrictMode>,
));
