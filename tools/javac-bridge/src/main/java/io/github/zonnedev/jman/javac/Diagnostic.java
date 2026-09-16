package io.github.zonnedev.jman.javac;

record Diagnostic(
    String kind,
    String code,
    long start,
    long end,
    long line,
    long column,
    String message) {}

