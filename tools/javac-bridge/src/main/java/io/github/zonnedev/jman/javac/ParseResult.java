package io.github.zonnedev.jman.javac;

import java.util.List;

record ParseResult(String packageName, List<TypeDeclaration> types, List<Diagnostic> diagnostics) {
  ParseResult {
    types = List.copyOf(types);
    diagnostics = List.copyOf(diagnostics);
  }
}

