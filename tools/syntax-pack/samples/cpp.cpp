#include <string>

// Says hello.
namespace quark {
template <typename T>
class Greeter {
public:
    std::string greet(const T &name) const { return "hello, " + name; }
};
}

auto script = R"js(console.log("hi"))js";

int main() {
    quark::Greeter<std::string> greeter;
    return greeter.greet("quark").empty() ? 1 : 0;
}
