package io.github.zonnedev.jman.javac;

import java.util.List;

record FormatResult(String source, List<Diagnostic> diagnostics) {}
