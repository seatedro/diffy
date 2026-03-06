#include <QTest>

#include "core/SyntaxTypes.h"
#include "core/syntax/Highlighter.h"
#include "core/syntax/LanguageRegistry.h"

class TestHighlighter : public QObject {
  Q_OBJECT

 private:
  diffy::LanguageRegistry registry_;
  diffy::Highlighter highlighter_;

 private slots:
  void initTestCase() {
    registry_.loadBuiltinGrammars();
  }

  void extensionMapping() {
    QVERIFY(registry_.grammarForExtension(".cpp") != nullptr);
    QVERIFY(registry_.grammarForExtension(".rs") != nullptr);
    QVERIFY(registry_.grammarForExtension(".py") != nullptr);
    QVERIFY(registry_.grammarForExtension(".nonexistent") == nullptr);
  }

  void cppKeyword() {
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
    const auto* grammar = registry_.grammarForExtension(".cpp");
    auto tokens = highlighter_.highlight(*grammar, "");
    QVERIFY(tokens.empty());
  }
};

QTEST_MAIN(TestHighlighter)
#include "test_highlighter.moc"
