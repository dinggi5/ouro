import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App";
import Settings from "./Settings";

// 창 이름으로 화면을 가른다 — 설정 창(개발 16)만 따로, 팝오버·크게 보기는 같은 App(App.tsx 가 다시 가른다).
const SETTINGS = getCurrentWebviewWindow().label === "settings";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{SETTINGS ? <Settings /> : <App />}</React.StrictMode>,
);
