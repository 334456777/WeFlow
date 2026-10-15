import React from 'react';
import { createRoot } from 'react-dom/client';
import { adoptToken } from './api/client';
import { App } from './App';
import './styles/tokens.css';
import './app.css';

adoptToken();

createRoot(document.getElementById('root')!).render(
  <React.StrictMode><App /></React.StrictMode>,
);
