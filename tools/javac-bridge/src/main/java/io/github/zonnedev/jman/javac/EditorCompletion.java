package io.github.zonnedev.jman.javac;

record EditorCompletion(
    String label, String kind, String detail, String insertText, String documentation) {}
