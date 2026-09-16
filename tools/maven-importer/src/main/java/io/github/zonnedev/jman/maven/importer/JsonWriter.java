package io.github.zonnedev.jman.maven.importer;

import java.util.List;

final class JsonWriter {
  private JsonWriter() {}

  static String write(MavenCompileModel model) {
    StringBuilder json = new StringBuilder(512);
    json.append('{');
    field(json, "schemaVersion", Integer.toString(model.schemaVersion()), false);
    stringField(json, "buildSystem", model.buildSystem());
    stringField(json, "projectPath", model.projectPath());
    stringField(json, "projectDirectory", model.projectDirectory());
    stringField(json, "taskPath", model.taskPath());
    listField(json, "sourceFiles", model.sourceFiles());
    listField(json, "sourceRoots", model.sourceRoots());
    listField(json, "classpath", model.classpath());
    listField(json, "modulePath", model.modulePath());
    listField(json, "projectDependencies", model.projectDependencies());
    listField(json, "annotationProcessorPath", model.annotationProcessorPath());
    listField(json, "annotationProcessorOptions", model.annotationProcessorOptions());
    listField(json, "compilerArgs", model.compilerArgs());
    nullableStringField(json, "release", model.release());
    nullableStringField(json, "encoding", model.encoding());
    nullableStringField(json, "generatedSourcesDirectory", model.generatedSourcesDirectory());
    listField(json, "generatedSourceDirectories", model.generatedSourceDirectories());
    nullableStringField(json, "destinationDirectory", model.destinationDirectory());
    nullableStringField(json, "javaCompilerExecutable", model.javaCompilerExecutable());
    nullableStringField(json, "javaLanguageVersion", model.javaLanguageVersion());
    return json.append('}').toString();
  }

  private static void stringField(StringBuilder json, String name, String value) {
    field(json, name, quote(value), true);
  }

  private static void nullableStringField(StringBuilder json, String name, String value) {
    field(json, name, value == null ? "null" : quote(value), true);
  }

  private static void listField(StringBuilder json, String name, List<String> values) {
    StringBuilder array = new StringBuilder("[");
    for (int index = 0; index < values.size(); index++) {
      if (index > 0) {
        array.append(',');
      }
      array.append(quote(values.get(index)));
    }
    field(json, name, array.append(']').toString(), true);
  }

  private static void field(
      StringBuilder json, String name, String encodedValue, boolean prependComma) {
    if (prependComma) {
      json.append(',');
    }
    json.append(quote(name)).append(':').append(encodedValue);
  }

  private static String quote(String value) {
    StringBuilder escaped = new StringBuilder(value.length() + 2).append('"');
    for (int index = 0; index < value.length(); index++) {
      char character = value.charAt(index);
      switch (character) {
        case '"' -> escaped.append("\\\"");
        case '\\' -> escaped.append("\\\\");
        case '\b' -> escaped.append("\\b");
        case '\f' -> escaped.append("\\f");
        case '\n' -> escaped.append("\\n");
        case '\r' -> escaped.append("\\r");
        case '\t' -> escaped.append("\\t");
        default -> {
          if (character < 0x20) {
            escaped.append(String.format("\\u%04x", (int) character));
          } else {
            escaped.append(character);
          }
        }
      }
    }
    return escaped.append('"').toString();
  }
}
