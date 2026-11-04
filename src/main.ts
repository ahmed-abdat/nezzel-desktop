import './app.css';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { mountWindow } from './windowEntry';

// Window identity is native; the query string is only a navigation hint.
// E2E only: `?window=capture-bar` renders another native window's page in the browser.
const e2eWindow = typeof __E2E__ !== 'undefined' && __E2E__ ? new URLSearchParams(location.search).get('window') : null;
const label = e2eWindow ?? (isTauri() ? getCurrentWindow().label : 'main');
void mountWindow(label);
