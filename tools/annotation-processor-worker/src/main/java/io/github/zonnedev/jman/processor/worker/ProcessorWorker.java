package io.github.zonnedev.jman.processor.worker;

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
import javax.tools.ToolProvider;

/** Persistent, line-oriented JVM worker for open-world annotation processors. */
public final class ProcessorWorker {
  public static final int PROTOCOL_VERSION = 2;

  private ProcessorWorker() {}

  public static void main(String[] arguments) throws IOException {
    if (arguments.length != 0) {
      throw new IllegalArgumentException("usage: ProcessorWorker");
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
        Result result = process(Path.of(line));
        output.println("OK\t" + result.generatedSources());
      } catch (Throwable failure) {
        String message =
            Base64.getEncoder()
                .encodeToString(failure.toString().getBytes(StandardCharsets.UTF_8));
        output.println("ERROR\t" + message);
      }
    }
  }

  static Result process(Path requestFile) throws IOException {
    Properties request = new Properties();
    try (var input = Files.newInputStream(requestFile)) {
      request.load(input);
    }
    int version = Integer.parseInt(required(request, "protocol.version"));
    if (version != PROTOCOL_VERSION) {
      throw new IllegalArgumentException("unsupported processor protocol " + version);
    }
    List<Path> sources = paths(request, "source");
    Path generatedTarget = Path.of(required(request, "generated.directory"));
    Path generated =
        generatedTarget.resolveSibling(generatedTarget.getFileName() + ".jman-java-staging");
    Path classesTarget = Path.of(required(request, "classes.directory"));
    Path classes = stagingClassesDirectory(classesTarget);
    Path partialClasses = partialClassesDirectory(classesTarget);
    cleanDirectory(generated);
    cleanDirectory(classes);
    cleanDirectory(partialClasses);
    Files.createDirectories(generated);
    Files.createDirectories(classes);

    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("jdk.compiler is unavailable");
    }
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    try (StandardJavaFileManager files =
        compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
      List<String> options = new ArrayList<>();
      options.add("-s");
      options.add(generated.toString());
      options.add("-d");
      options.add(classes.toString());
      addPathOption(options, "-classpath", paths(request, "classpath"));
      addPathOption(options, "-sourcepath", paths(request, "source.path"));
      addPathOption(options, "-processorpath", paths(request, "processor.path"));
      String release = request.getProperty("release");
      if (release != null && !release.isBlank()) {
        options.add("--release");
        options.add(release);
      }
      options.addAll(values(request, "processor.option"));
      boolean success =
          compiler
              .getTask(
                  null,
                  files,
                  diagnostics,
                  options,
                  null,
                  files.getJavaFileObjectsFromPaths(sources))
              .call();
      if (!success) {
        publishDirectory(classes, partialClasses);
        cleanDirectory(generated);
        String encoded =
            diagnostics.getDiagnostics().stream()
                .map(diagnostic -> diagnostic.getCode() + ": " + diagnostic.getMessage(Locale.ROOT))
                .reduce((left, right) -> left + "\n" + right)
                .orElse("annotation processing failed");
        throw new IllegalStateException(encoded);
      }
    }
    long generatedCount;
    try (Stream<Path> paths = Files.walk(generated)) {
      generatedCount = paths.filter(path -> path.toString().endsWith(".java")).count();
    }
    publishDirectory(generated, generatedTarget);
    publishDirectory(classes, classesTarget);
    return new Result(generatedCount);
  }

  static Path stagingClassesDirectory(Path target) {
    return stateClassesDirectory(target, "staging", ".jman-java-staging");
  }

  static Path partialClassesDirectory(Path target) {
    return stateClassesDirectory(target, "partial", ".jman-java-partial");
  }

  private static Path stateClassesDirectory(Path target, String state, String legacySuffix) {
    for (int index = 0; index < target.getNameCount(); index++) {
      if (target.getName(index).toString().equals("current")) {
        Path directory = target.getRoot();
        if (directory == null) {
          directory = Path.of("");
        }
        for (int component = 0; component < target.getNameCount(); component++) {
          directory =
              directory.resolve(
                  component == index ? state : target.getName(component).toString());
        }
        return directory;
      }
    }
    return target.resolveSibling(target.getFileName() + legacySuffix);
  }

  private static void publishDirectory(Path source, Path target) throws IOException {
    cleanDirectory(target);
    Path parent = target.getParent();
    if (parent != null) {
      Files.createDirectories(parent);
    }
    Files.move(source, target);
  }

  private static void addPathOption(List<String> options, String name, List<Path> paths) {
    if (!paths.isEmpty()) {
      options.add(name);
      options.add(String.join(java.io.File.pathSeparator, paths.stream().map(Path::toString).toList()));
    }
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
      throw new IllegalArgumentException("missing processor request property " + name);
    }
    return value;
  }

  private static void cleanDirectory(Path directory) throws IOException {
    if (!Files.exists(directory)) {
      return;
    }
    try (Stream<Path> paths = Files.walk(directory)) {
      for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
        Files.delete(path);
      }
    }
  }

  record Result(long generatedSources) {}
}
