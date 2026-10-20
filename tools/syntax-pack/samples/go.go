package main

import "fmt"

// Greeter says hello.
type Greeter struct{ Name string }

func (g Greeter) Greet() string {
	return fmt.Sprintf("hello, %s", g.Name)
}

func main() {
	fmt.Println(Greeter{Name: "quark"}.Greet(), 42)
}
