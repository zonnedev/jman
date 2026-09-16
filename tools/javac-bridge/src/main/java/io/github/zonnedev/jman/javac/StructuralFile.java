package io.github.zonnedev.jman.javac;

import java.util.List;

record StructuralFile(
    String fileName,
    String packageName,
    List<String> imports,
    List<SemanticSymbol> symbols,
    List<Diagnostic> diagnostics) {
  StructuralFile {
    imports = List.copyOf(imports);
    symbols = List.copyOf(symbols);
    diagnostics = List.copyOf(diagnostics);
  }
}
