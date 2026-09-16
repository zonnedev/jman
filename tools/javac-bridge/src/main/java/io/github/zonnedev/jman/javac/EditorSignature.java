package io.github.zonnedev.jman.javac;

import java.util.List;

record EditorSignature(
    String label, List<String> parameters, String returnType, String documentation) {}
