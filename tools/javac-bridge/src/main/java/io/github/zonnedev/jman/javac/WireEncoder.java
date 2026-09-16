package io.github.zonnedev.jman.javac;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;

final class WireEncoder {
  private WireEncoder() {}

  static byte[] encode(ParseResult result) {
    try {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      DataOutputStream output = new DataOutputStream(bytes);
      output.writeBytes("JFE1");
      writeString(output, result.packageName());
      output.writeInt(result.types().size());
      for (TypeDeclaration type : result.types()) {
        writeString(output, type.kind());
        writeString(output, type.name());
        output.writeLong(type.start());
        output.writeLong(type.end());
      }
      output.writeInt(result.diagnostics().size());
      for (Diagnostic diagnostic : result.diagnostics()) {
        writeString(output, diagnostic.kind());
        writeString(output, diagnostic.code());
        output.writeLong(diagnostic.start());
        output.writeLong(diagnostic.end());
        output.writeLong(diagnostic.line());
        output.writeLong(diagnostic.column());
        writeString(output, diagnostic.message());
      }
      output.flush();
      return bytes.toByteArray();
    } catch (IOException impossible) {
      throw new AssertionError("In-memory encoding failed", impossible);
    }
  }

  static byte[] encode(SemanticResult result) {
    try {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      DataOutputStream output = new DataOutputStream(bytes);
      output.writeBytes("JFS1");
      writeString(output, result.packageName());
      output.writeInt(result.symbols().size());
      for (SemanticSymbol symbol : result.symbols()) {
        writeString(output, symbol.role());
        writeString(output, symbol.kind());
        writeString(output, symbol.name());
        writeString(output, symbol.qualifiedName());
        writeString(output, symbol.symbolId());
        output.writeLong(symbol.start());
        output.writeLong(symbol.end());
      }
      writeDiagnostics(output, result.diagnostics());
      output.flush();
      return bytes.toByteArray();
    } catch (IOException impossible) {
      throw new AssertionError("In-memory encoding failed", impossible);
    }
  }

  static byte[] encode(WorkspaceParseResult result) {
    try {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      DataOutputStream output = new DataOutputStream(bytes);
      output.writeBytes("JFB1");
      output.writeInt(result.files().size());
      for (StructuralFile file : result.files()) {
        writeString(output, file.fileName());
        writeString(output, file.packageName());
        output.writeInt(file.imports().size());
        for (String imported : file.imports()) {
          writeString(output, imported);
        }
        output.writeInt(file.symbols().size());
        for (SemanticSymbol symbol : file.symbols()) {
          writeString(output, symbol.role());
          writeString(output, symbol.kind());
          writeString(output, symbol.name());
          writeString(output, symbol.qualifiedName());
          writeString(output, symbol.symbolId());
          output.writeLong(symbol.start());
          output.writeLong(symbol.end());
        }
        writeDiagnostics(output, file.diagnostics());
      }
      output.flush();
      return bytes.toByteArray();
    } catch (IOException impossible) {
      throw new AssertionError("In-memory encoding failed", impossible);
    }
  }

  static byte[] encode(EditorQueryResult result) {
    try {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      DataOutputStream output = new DataOutputStream(bytes);
      output.writeBytes("JFQ2");
      output.writeInt(result.completions().size());
      for (EditorCompletion completion : result.completions()) {
        writeString(output, completion.label());
        writeString(output, completion.kind());
        writeString(output, completion.detail());
        writeString(output, completion.insertText());
        writeString(output, completion.documentation());
      }
      output.writeInt(result.signatures().size());
      for (EditorSignature signature : result.signatures()) {
        writeString(output, signature.label());
        output.writeInt(signature.parameters().size());
        for (String parameter : signature.parameters()) {
          writeString(output, parameter);
        }
        writeString(output, signature.returnType());
        writeString(output, signature.documentation());
      }
      output.writeBoolean(result.hover() != null);
      if (result.hover() != null) {
        writeString(output, result.hover().detail());
        writeString(output, result.hover().documentation());
      }
      output.writeBoolean(result.definition() != null);
      if (result.definition() != null) {
        EditorDefinition definition = result.definition();
        writeString(output, definition.symbolId());
        writeString(output, definition.module());
        writeString(output, definition.owner());
        writeString(output, definition.name());
        writeString(output, definition.descriptor());
        writeString(output, definition.sourceName());
        writeString(output, definition.source());
        output.writeLong(definition.start());
        output.writeLong(definition.end());
        output.writeBoolean(definition.decompiled());
      }
      output.writeBoolean(result.typeDefinition() != null);
      if (result.typeDefinition() != null) {
        EditorDefinition definition = result.typeDefinition();
        writeString(output, definition.symbolId());
        writeString(output, definition.module());
        writeString(output, definition.owner());
        writeString(output, definition.name());
        writeString(output, definition.descriptor());
        writeString(output, definition.sourceName());
        writeString(output, definition.source());
        output.writeLong(definition.start());
        output.writeLong(definition.end());
        output.writeBoolean(definition.decompiled());
      }
      output.flush();
      return bytes.toByteArray();
    } catch (IOException impossible) {
      throw new AssertionError("In-memory encoding failed", impossible);
    }
  }

  private static void writeDiagnostics(DataOutputStream output, java.util.List<Diagnostic> diagnostics)
      throws IOException {
    output.writeInt(diagnostics.size());
    for (Diagnostic diagnostic : diagnostics) {
      writeString(output, diagnostic.kind());
      writeString(output, diagnostic.code());
      output.writeLong(diagnostic.start());
      output.writeLong(diagnostic.end());
      output.writeLong(diagnostic.line());
      output.writeLong(diagnostic.column());
      writeString(output, diagnostic.message());
    }
  }

  private static void writeString(DataOutputStream output, String value) throws IOException {
    byte[] encoded = value.getBytes(StandardCharsets.UTF_8);
    output.writeInt(encoded.length);
    output.write(encoded);
  }
}
