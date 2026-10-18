// Angle brackets here are a type assertion, never a tag.
interface Greeter {
  name: string;
}

export function greet(value: unknown): string {
  const greeter = <Greeter>value;
  return `hello, ${greeter.name}`;
}
