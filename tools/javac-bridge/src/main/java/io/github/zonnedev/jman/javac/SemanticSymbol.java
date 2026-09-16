package io.github.zonnedev.jman.javac;

record SemanticSymbol(
    String role,
    String kind,
    String name,
    String qualifiedName,
    String symbolId,
    long start,
    long end) {}
