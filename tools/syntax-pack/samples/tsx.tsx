// A generic arrow function and an element in one file.
type Props = { name: string };

const first = <T,>(items: T[]): T => items[0];

export function Greeting({ name }: Props) {
  return <div className="greeting">hello, {first([name])}</div>;
}
