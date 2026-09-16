package io.github.zonnedev.jman.javac;

import java.util.List;

record WorkspaceParseResult(List<StructuralFile> files) {
  WorkspaceParseResult {
    files = List.copyOf(files);
  }
}
