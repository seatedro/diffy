#include <QTest>

#include <cstdlib>

#include "core/SyntaxTypes.h"
#include "core/syntax/Highlighter.h"
#include "core/syntax/LanguageRegistry.h"

class TestHighlighter : public QObject {
  Q_OBJECT

 private:
  diffy::LanguageRegistry registry_;
  diffy::Highlighter highlighter_;

  bool hasGrammars_ = false;

 private slots:
  void initTestCase() {
    const char* paths = std::getenv("DIFFY_GRAMMAR_PATHS");
    if (paths != nullptr && paths[0] != '\0') {
      registry_.discoverGrammars(paths);
      hasGrammars_ = registry_.grammarForName("cpp") != nullptr;
    }
    if (!hasGrammars_) {
      qWarning("DIFFY_GRAMMAR_PATHS not set or no cpp grammar found; skipping grammar-dependent tests");
    }
  }

  void extensionMapping() {
    if (!hasGrammars_) {
      QSKIP("No grammars available");
    }
    QVERIFY(registry_.grammarForExtension(".cpp") != nullptr);
    QVERIFY(registry_.grammarForExtension(".rs") != nullptr);
    QVERIFY(registry_.grammarForExtension(".py") != nullptr);
    QVERIFY(registry_.grammarForExtension(".nonexistent") == nullptr);
  }

  void cppKeyword() {
    if (!hasGrammars_) {
      QSKIP("No grammars available");
    }
    const auto* grammar = registry_.grammarForExtension(".cpp");
    QVERIFY(grammar != nullptr);

    auto tokens = highlighter_.highlight(*grammar, "int main() { return 0; }");
    QVERIFY(!tokens.empty());

    bool foundKeyword = false;
    bool foundNumber = false;
    for (const auto& tok : tokens) {
      if (tok.syntaxKind == diffy::SyntaxTokenKind::Keyword || tok.syntaxKind == diffy::SyntaxTokenKind::Type) {
        foundKeyword = true;
      }
      if (tok.syntaxKind == diffy::SyntaxTokenKind::Number) {
        foundNumber = true;
      }
    }
    QVERIFY(foundKeyword);
    QVERIFY(foundNumber);
  }

  void pythonString() {
    if (!hasGrammars_) {
      QSKIP("No grammars available");
    }
    const auto* grammar = registry_.grammarForExtension(".py");
    QVERIFY(grammar != nullptr);

    auto tokens = highlighter_.highlight(*grammar, "x = \"hello world\"");
    QVERIFY(!tokens.empty());

    bool foundString = false;
    for (const auto& tok : tokens) {
      if (tok.syntaxKind == diffy::SyntaxTokenKind::String) {
        foundString = true;
        break;
      }
    }
    QVERIFY(foundString);
  }

  void emptySource() {
    if (!hasGrammars_) {
      QSKIP("No grammars available");
    }
    const auto* grammar = registry_.grammarForExtension(".cpp");
    auto tokens = highlighter_.highlight(*grammar, "");
    QVERIFY(tokens.empty());
  }
};

QTEST_MAIN(TestHighlighter)
#include "test_highlighter.moc"
