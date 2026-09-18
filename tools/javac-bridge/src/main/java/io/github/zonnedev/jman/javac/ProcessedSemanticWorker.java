package io.github.zonnedev.jman.javac;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.Properties;
import java.util.stream.Stream;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

/**
 * Persistent JVM lane for semantics that require open-world annotation processors.
 *
 * <p>The worker deliberately knows nothing about individual processors. It runs the processor path
 * exported by the build and extracts semantics from that same attributed javac task.
 */
public final class ProcessedSemanticWorker {
  public static final int PROTOCOL_VERSION = 1;

  private ProcessedSemanticWorker() {}

  public static void main(String[] arguments) throws IOException {
    if (arguments.length != 0) {
      throw new IllegalArgumentException("usage: ProcessedSemanticWorker");
    }
    BufferedReader input =
        new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
    PrintWriter output = new PrintWriter(System.out, true, StandardCharsets.UTF_8);
    output.println("READY\t" + PROTOCOL_VERSION);
    for (String line; (line = input.readLine()) != null; ) {
      if (line.equals("STOP")) {
        output.println("STOPPED");
        return;
      }
      try {
        byte[] result = execute(Path.of(line));
        output.println("OK\t" + Base64.getEncoder().encodeToString(result));
      } catch (Throwable failure) {
        String message = stackMessage(failure);
        output.println(
            "ERROR\t"
                + Base64.getEncoder()
                    .encodeToString(message.getBytes(StandardCharsets.UTF_8)));
      }
    }
  }

  static byte[] execute(Path requestFile) throws IOException {
    Properties request = new Properties();
    try (var input = Files.newBufferedReader(requestFile, StandardCharsets.UTF_8)) {
      request.load(input);
    }
    int version = Integer.parseInt(required(request, "protocol.version"));
    if (version != PROTOCOL_VERSION) {
      throw new IllegalArgumentException("unsupported processed semantic protocol " + version);
    }
    String operation = required(request, "operation");
    String fileName = required(request, "file.name");
    String source =
        new String(
            Base64.getDecoder().decode(required(request, "source.base64")),
            StandardCharsets.UTF_8);
    int release = Integer.parseInt(required(request, "release"));
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("jdk.compiler is unavailable");
    }

    Path outputRoot = Files.createTempDirectory("jman-processed-semantics-");
    try {
      DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
      try (StandardJavaFileManager files =
          compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
        setLocation(files, StandardLocation.CLASS_PATH, paths(request, "classpath"));
        setLocation(files, StandardLocation.MODULE_PATH, paths(request, "module.path"));
        setLocation(files, StandardLocation.SOURCE_PATH, existingPaths(request, "source.path"));
        setLocation(
            files,
            StandardLocation.ANNOTATION_PROCESSOR_PATH,
            paths(request, "processor.path"));
        Path classes = outputRoot.resolve("classes");
        Path generated = outputRoot.resolve("generated");
        Files.createDirectories(classes);
        Files.createDirectories(generated);
        List<String> options = new ArrayList<>();
        options.add("-proc:full");
        options.add("-d");
        options.add(classes.toString());
        options.add("-s");
        options.add(generated.toString());
        options.addAll(values(request, "compiler.option"));
        options.addAll(values(request, "processor.option"));
        if (operation.equals("analyze")) {
          return WireEncoder.encode(
              JavacFrontend.analyze(
                  compiler,
                  files,
                  diagnostics,
                  fileName,
                  source,
                  release,
                  List.of(),
                  options));
        }
        if (operation.equals("query")) {
          int cursor = Integer.parseInt(required(request, "cursor"));
          return WireEncoder.encode(
              EditorQueries.query(
                  compiler, files, fileName, source, cursor, release, options));
        }
        throw new IllegalArgumentException("unsupported processed semantic operation " + operation);
      }
    } finally {
      deleteTree(outputRoot);
    }
  }

  private static void setLocation(
      StandardJavaFileManager files, StandardLocation location, List<Path> paths)
      throws IOException {
    if (!paths.isEmpty()) {
      files.setLocationFromPaths(location, paths);
    }
  }

  private static List<Path> existingPaths(Properties request, String prefix) {
    return paths(request, prefix).stream().filter(Files::exists).toList();
  }

  private static List<Path> paths(Properties request, String prefix) {
    return values(request, prefix).stream().map(Path::of).toList();
  }

  private static List<String> values(Properties request, String prefix) {
    int count = Integer.parseInt(request.getProperty(prefix + ".count", "0"));
    List<String> values = new ArrayList<>(count);
    for (int index = 0; index < count; index++) {
      values.add(required(request, prefix + "." + index));
    }
    return List.copyOf(values);
  }

  private static String required(Properties request, String name) {
    String value = request.getProperty(name);
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException("missing processed semantic request property " + name);
    }
    return value;
  }

  private static String stackMessage(Throwable failure) {
    StringBuilder message = new StringBuilder(failure.toString());
    for (StackTraceElement frame : failure.getStackTrace()) {
      message.append("\n\tat ").append(frame);
      if (message.length() > 16_384) {
        message.append("\n\t...");
        break;
      }
    }
    return message.toString();
  }

  private static void deleteTree(Path root) throws IOException {
    if (!Files.exists(root)) {
      return;
    }
    try (Stream<Path> paths = Files.walk(root)) {
      for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }
}
