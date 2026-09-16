package io.github.zonnedev.jman.javac;

import java.util.List;

record SemanticResult(
    String packageName, List<SemanticSymbol> symbols, List<Diagnostic> diagnostics) {
  SemanticResult {
    symbols = List.copyOf(symbols);
    diagnostics = List.copyOf(diagnostics);
  }
}
