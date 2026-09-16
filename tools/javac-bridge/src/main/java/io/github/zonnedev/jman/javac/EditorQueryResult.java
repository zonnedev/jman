package io.github.zonnedev.jman.javac;

import java.util.List;

record EditorQueryResult(
    List<EditorCompletion> completions,
    List<EditorSignature> signatures,
    EditorHover hover,
    EditorDefinition definition,
    EditorDefinition typeDefinition) {}
