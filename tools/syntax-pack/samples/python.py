import os


class Greeter:
    """Says hello."""

    def greet(self, name: str) -> str:
        # Formats the greeting.
        return f"hello, {name}" if name else None


print(Greeter().greet(os.getlogin()), 42)
