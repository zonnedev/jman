package io.github.zonnedev.jman.javac;

record EditorDefinition(
    String symbolId,
    String module,
    String owner,
    String name,
    String descriptor,
    String sourceName,
    String source,
    long start,
    long end,
    boolean decompiled) {}
