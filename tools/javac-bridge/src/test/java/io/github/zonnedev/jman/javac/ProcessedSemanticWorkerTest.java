package io.github.zonnedev.jman.javac;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Base64;
import java.util.List;
import java.util.Properties;
import java.util.Set;
import javax.annotation.processing.AbstractProcessor;
import javax.annotation.processing.RoundEnvironment;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;

public final class ProcessedSemanticWorkerTest {
  private ProcessedSemanticWorkerTest() {}

  public static void main(String[] arguments) throws Exception {
    Path root = Files.createTempDirectory("jman-processed-worker-test-");
    try {
      String source =
          """
          package fixture;
          @interface Generate {}
          @Generate final class Example {
            Generated generated = new Generated();
            String value() { return generated.value(); }
          }
          """;
      byte[] semantic =
          ProcessedSemanticWorker.execute(
              request(root.resolve("analyze.properties"), "analyze", source, 0));
      assert startsWith(semantic, "JFS1") : "semantic response has the wrong wire version";
      assert contains(semantic, "fixture.Generated")
          : "generated type was not visible in the processed semantic model";
      assert !contains(semantic, "cant.resolve")
          : "generated type produced an unresolved-symbol diagnostic";

      int cursor = source.indexOf("generated.value") + "generated.val".length();
      byte[] query =
          ProcessedSemanticWorker.execute(
              request(root.resolve("query.properties"), "query", source, cursor));
      assert startsWith(query, "JFQ2") : "query response has the wrong wire version";
      assert contains(query, "value")
          : "generated member was absent from processed completion results";
    } finally {
      deleteTree(root);
    }
  }

  private static Path request(Path path, String operation, String source, int cursor)
      throws Exception {
    Path classes =
        Path.of(
            ProcessedSemanticWorkerTest.class
                .getProtectionDomain()
                .getCodeSource()
                .getLocation()
                .toURI());
    Properties request = new Properties();
    request.setProperty(
        "protocol.version", Integer.toString(ProcessedSemanticWorker.PROTOCOL_VERSION));
    request.setProperty("operation", operation);
    request.setProperty("file.name", "Example.java");
    request.setProperty(
        "source.base64",
        Base64.getEncoder().encodeToString(source.getBytes(StandardCharsets.UTF_8)));
    request.setProperty("release", "17");
    request.setProperty("cursor", Integer.toString(cursor));
    list(request, "classpath", List.of(classes.toString()));
    list(request, "module.path", List.of());
    list(request, "source.path", List.of());
    list(request, "processor.path", List.of(classes.toString()));
    list(
        request,
        "processor.option",
        List.of("-processor", GeneratedTypeProcessor.class.getName()));
    list(request, "compiler.option", List.of());
    try (var output = Files.newOutputStream(path)) {
      request.store(output, null);
    }
    return path;
  }

  private static void list(Properties properties, String name, java.util.List<String> values) {
    properties.setProperty(name + ".count", Integer.toString(values.size()));
    for (int index = 0; index < values.size(); index++) {
      properties.setProperty(name + "." + index, values.get(index));
    }
  }

  private static boolean startsWith(byte[] bytes, String text) {
    return new String(bytes, 0, Math.min(bytes.length, text.length()), StandardCharsets.UTF_8)
        .equals(text);
  }

  private static boolean contains(byte[] bytes, String text) {
    return new String(bytes, StandardCharsets.UTF_8).contains(text);
  }

  private static void deleteTree(Path root) throws IOException {
    if (!Files.exists(root)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(java.util.Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }

  public static final class GeneratedTypeProcessor extends AbstractProcessor {
    private boolean generated;

    @Override
    public Set<String> getSupportedAnnotationTypes() {
      return Set.of("*");
    }

    @Override
    public SourceVersion getSupportedSourceVersion() {
      return SourceVersion.latestSupported();
    }

    @Override
    public boolean process(
        Set<? extends TypeElement> annotations, RoundEnvironment roundEnvironment) {
      if (generated || roundEnvironment.processingOver()) return false;
      generated = true;
      try (var writer = processingEnv.getFiler().createSourceFile("fixture.Generated").openWriter()) {
        writer.write(
            "package fixture; public final class Generated { public String value() { return \"\"; } }");
      } catch (IOException failure) {
        throw new IllegalStateException(failure);
      }
      return false;
    }
  }
}
