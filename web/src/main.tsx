// 最先导入：tokens.css 声明层叠层的顺序，这个声明只有在任何层的规则之前才有效
import './styles/tokens.css';
// 默认皮肤：页面使用的颜色和角色（#828）
import './styles/neumorphism.css';
import './app.css';
import './styles/roles.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { adoptToken } from './api/client';
import { App } from './App';
import { Backdrop } from './Backdrop';
import { languageReady } from './i18n';
import { skinReady } from './skin';

adoptToken();

// 一开始就用上次访问时的语言和皮肤，而不是先显示中文和默认皮肤再切换
void Promise.all([languageReady(), skinReady()]).then(() => createRoot(document.getElementById('root')!).render(
  <StrictMode><Backdrop /><App /></StrictMode>,
));
