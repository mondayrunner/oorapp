import { createApp } from "vue";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import App from "./App.vue";
import Hud from "./components/Hud.vue";
import "./style.css";

// The same page serves the main window and the note pill.
const hud = getCurrentWebviewWindow().label === "hud";
createApp(hud ? Hud : App).mount("#app");
