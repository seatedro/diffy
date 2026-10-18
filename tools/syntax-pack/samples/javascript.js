// Renders a greeting.
import { render } from "./ui.js";

export function greet(name) {
  const count = 3;
  return <p className="greeting">hello, {name} x{count}</p>;
}

render(greet("quark"));
