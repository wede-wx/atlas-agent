import "./styles.css";
import { mount } from "./app";

const root = document.getElementById("app");
if (root) {
  void mount(root);
}
