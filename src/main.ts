import "./styles/tokens.css";
import "./styles/base.css";
import { mount } from "svelte";
import App from "./App.svelte";

mount(App, { target: document.getElementById("app")! });

// Dev-only accessibility harness: in the browser console run  await __axe.run()
if (import.meta.env.DEV) import("axe-core").then((m) => ((window as unknown as { __axe: unknown }).__axe = m.default));
