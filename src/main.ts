import "./styles.css";
import { mount } from "./app";

const root = document.getElementById("app");
if (root) {
  void mount(root);
}

// Note: a 252×52 always-on-top "float" window route (/#float) exists in
// tauri.conf.json. This baseline serves the main window only; a float UI can be
// added by branching on `location.hash === "#float"` here.
