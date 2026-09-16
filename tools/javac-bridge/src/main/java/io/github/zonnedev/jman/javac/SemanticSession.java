package io.github.zonnedev.jman.javac;

import java.io.IOException;
import java.nio.file.Path;
import java.util.List;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

final class SemanticSession implements AutoCloseable {
  private static final int MAX_CACHED_DOCUMENTS = 128;

  private final int release;
  private final JavaCompiler compiler;
  private final StandardJavaFileManager files;
  private final List<Path> classpath;
  private final List<Path> modulePath;
  private final Path moduleInfo;
  private final Path moduleOverlay;
  private final Path moduleQueryOverlay;
  private final List<Path> moduleQuerySourcePath;
  private final List<String> compilerOptions;
  private final List<Diagnostic> modulePathDiagnostics;
  private long analysisCount;
  private final Map<String, CachedAnalysis> analyses =
      new LinkedHashMap<>(16, 0.75f, true) {
        @Override
        protected boolean removeEldestEntry(Map.Entry<String, CachedAnalysis> eldest) {
          return size() > MAX_CACHED_DOCUMENTS;
        }
      };

  SemanticSession(
      List<Path> classpath,
      List<Path> modulePath,
      List<Path> sourcePath,
      Path moduleInfo,
      List<String> compilerOptions,
      int release) {
    this.release = release;
    this.moduleInfo = moduleInfo;
    this.classpath = List.copyOf(classpath);
    this.modulePath = List.copyOf(modulePath);
    this.modulePathDiagnostics = validateModulePath(modulePath);
    Path overlay = null;
    Path queryOverlay = null;
    String moduleName = null;
    java.util.ArrayList<String> options = new java.util.ArrayList<>(compilerOptions);
    Map<String, java.util.LinkedHashSet<String>> moduleSources = new LinkedHashMap<>();
    for (int index = 0; index < options.size(); ) {
      String option = options.get(index);
      String value = null;
      int remove = 1;
      if ("--module-source-path".equals(option) && index + 1 < options.size()) {
        value = options.get(index + 1);
        remove = 2;
      } else if (option.startsWith("--module-source-path=")) {
        value = option.substring("--module-source-path=".length());
      }
      if (value == null) {
        index++;
        continue;
      }
      addModuleSource(moduleSources, value);
      for (int count = 0; count < remove; count++) options.remove(index);
    }
    if (moduleInfo != null) {
      String descriptor;
      try {
        descriptor = java.nio.file.Files.readString(moduleInfo);
      } catch (IOException exception) {
        throw new IllegalArgumentException("Unable to read " + moduleInfo, exception);
      }
      java.util.regex.Matcher declaration =
          java.util.regex.Pattern.compile("\\b(?:open\\s+)?module\\s+([\\w.]+)")
              .matcher(descriptor);
      if (declaration.find()) {
        moduleName = declaration.group(1);
        try {
          overlay = java.nio.file.Files.createTempDirectory("jman-java-module-overlay-");
        } catch (IOException exception) {
          throw new IllegalArgumentException("Unable to create module overlay", exception);
        }
        Path output = overlay.resolve(".classes");
        try {
          java.nio.file.Files.createDirectories(output);
        } catch (IOException exception) {
          throw new IllegalArgumentException("Unable to create module output", exception);
        }
        options.add("-d");
        options.add(output.toString());
        java.util.LinkedHashSet<String> ownerSources =
            moduleSources.computeIfAbsent(moduleName, ignored -> new java.util.LinkedHashSet<>());
        ownerSources.add(overlay.toString());
        populateOverlay(overlay, sourcePath);
        try {
          queryOverlay = java.nio.file.Files.createTempDirectory("jman-java-module-query-overlay-");
          populateOverlay(queryOverlay, sourcePath);
          java.nio.file.Files.deleteIfExists(queryOverlay.resolve("module-info.java"));
        } catch (IOException exception) {
          if (queryOverlay != null) deleteOverlay(queryOverlay);
          throw new IllegalArgumentException("Unable to create module query overlay", exception);
        }
        Path overlayDescriptor = overlay.resolve("module-info.java");
        try {
          java.nio.file.Files.deleteIfExists(overlayDescriptor);
          java.nio.file.Files.createSymbolicLink(overlayDescriptor, moduleInfo);
        } catch (UnsupportedOperationException | IOException failure) {
          try {
            java.nio.file.Files.copy(
                moduleInfo,
                overlayDescriptor,
                java.nio.file.StandardCopyOption.REPLACE_EXISTING);
          } catch (IOException exception) {
            throw new IllegalArgumentException(
                "Unable to populate module descriptor overlay from " + moduleInfo, exception);
          }
        }
      }
    }
    String ownerModuleName = moduleName;
    moduleSources.forEach(
        (module, paths) -> {
          if (!module.equals(ownerModuleName)) {
            options.add("--module-source-path");
            options.add(module + "=" + String.join(java.io.File.pathSeparator, paths));
          }
        });
    this.moduleOverlay = overlay;
    this.moduleQueryOverlay = queryOverlay;
    java.util.ArrayList<Path> querySourcePath = new java.util.ArrayList<>();
    if (queryOverlay != null) querySourcePath.add(queryOverlay);
    sourcePath.stream()
        .filter(java.nio.file.Files::isRegularFile)
        .filter(path -> !querySourcePath.contains(path))
        .forEach(querySourcePath::add);
    this.moduleQuerySourcePath = List.copyOf(querySourcePath);
    this.compilerOptions = List.copyOf(options);
    compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("The jdk.compiler module is unavailable");
    }
    files = compiler.getStandardFileManager(null, Locale.ROOT, null);
    try {
      if (!classpath.isEmpty()) {
        files.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath);
      }
      if (!sourcePath.isEmpty() && moduleInfo == null) {
        files.setLocationFromPaths(
            StandardLocation.SOURCE_PATH,
            sourcePath.stream().filter(java.nio.file.Files::exists).toList());
      }
      if (!modulePath.isEmpty()) {
        files.setLocationFromPaths(StandardLocation.MODULE_PATH, modulePath);
      }
    } catch (IOException exception) {
      throw new IllegalArgumentException("Unable to configure semantic session paths", exception);
    }
  }

  synchronized SemanticResult analyze(String fileName, String source) {
    CachedAnalysis cached = analyses.get(fileName);
    if (cached != null && cached.source().equals(source)) {
      return cached.result();
    }
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    analysisCount++;
    SemanticResult result;
    if (moduleOverlay == null) {
      result =
          JavacFrontend.analyze(
              compiler,
              files,
              diagnostics,
              fileName,
              source,
              release,
              List.of(),
              compilerOptions);
    } else {
      String packageName = "";
      java.util.regex.Matcher packageDeclaration =
          java.util.regex.Pattern.compile("\\bpackage\\s+([\\w.]+)\\s*;").matcher(source);
      if (packageDeclaration.find()) packageName = packageDeclaration.group(1);
      Path relative =
          packageName.isEmpty()
              ? Path.of(fileName).getFileName()
              : Path.of(packageName.replace('.', '/')).resolve(Path.of(fileName).getFileName());
      Path overlayFile = moduleOverlay.resolve(relative);
      try {
        java.nio.file.Files.createDirectories(overlayFile.getParent());
        if (java.nio.file.Files.isSymbolicLink(overlayFile)) {
          java.nio.file.Files.delete(overlayFile);
        }
        java.nio.file.Files.writeString(overlayFile, source);
      } catch (IOException exception) {
        throw new IllegalStateException("Unable to update module overlay " + overlayFile, exception);
      }
      try (StandardJavaFileManager analysisFiles =
          compiler.getStandardFileManager(null, Locale.ROOT, null)) {
        configureLocations(analysisFiles, classpath, modulePath);
        analysisFiles.setLocationFromPaths(
            StandardLocation.SOURCE_PATH, List.of(moduleOverlay));
        boolean descriptor =
            Path.of(fileName).getFileName().toString().equals("module-info.java");
        List<JavaFileObject> units = new java.util.ArrayList<>();
        JavaFileObject primary =
            analysisFiles.getJavaFileObjectsFromPaths(List.of(overlayFile)).iterator().next();
        if (descriptor) {
          try (var sources = java.nio.file.Files.walk(moduleOverlay)) {
            analysisFiles
                .getJavaFileObjectsFromPaths(
                    sources
                        .filter(java.nio.file.Files::isRegularFile)
                        .filter(path -> path.toString().endsWith(".java"))
                        .filter(path -> !path.equals(overlayFile))
                        .toList())
                .forEach(units::add);
          }
        } else {
          Path descriptorFile = moduleOverlay.resolve("module-info.java");
          if (java.nio.file.Files.isRegularFile(descriptorFile)) {
            analysisFiles
                .getJavaFileObjectsFromPaths(List.of(descriptorFile))
                .forEach(units::add);
          }
          analysisFiles
              .getJavaFileObjectsFromPaths(
                  companionSources(moduleOverlay, overlayFile, packageName, source))
              .forEach(units::add);
        }
        try {
          result =
              JavacFrontend.analyze(
                  compiler,
                  analysisFiles,
                  diagnostics,
                  primary,
                  source,
                  release,
                  units,
                  compilerOptions);
        } catch (IllegalStateException failure) {
          if (!causedByAssertion(failure)) throw failure;
          // javac 25 can assert in Enter.moduleEnv for valid editor overlays.
          // Preserve language features by retrying as an unnamed-module
          // attribution task; JPMS diagnostics remain available from
          // module-info.java analysis itself.
          DiagnosticCollector<JavaFileObject> fallbackDiagnostics = new DiagnosticCollector<>();
          try (StandardJavaFileManager fallbackFiles =
              compiler.getStandardFileManager(null, Locale.ROOT, null)) {
            java.util.ArrayList<Path> fallbackClasspath = new java.util.ArrayList<>(classpath);
            fallbackClasspath.addAll(modulePath);
            if (!fallbackClasspath.isEmpty()) {
              fallbackFiles.setLocationFromPaths(
                  StandardLocation.CLASS_PATH,
                  fallbackClasspath.stream().filter(java.nio.file.Files::exists).toList());
            }
            fallbackFiles.setLocationFromPaths(
                StandardLocation.SOURCE_PATH, List.of(moduleOverlay));
            List<JavaFileObject> fallbackUnits =
                units.stream()
                    .filter(
                        unit ->
                            !Path.of(unit.getName())
                                .getFileName()
                                .toString()
                                .equals("module-info.java"))
                    .toList();
            result =
                JavacFrontend.analyze(
                    compiler,
                    fallbackFiles,
                    fallbackDiagnostics,
                    primary,
                    source,
                    release,
                    fallbackUnits,
                    List.of());
            diagnostics = fallbackDiagnostics;
          }
        }
      } catch (IOException exception) {
        throw new IllegalStateException("Unable to analyze modular source", exception);
      }
    }
    if (Path.of(fileName).getFileName().toString().equals("module-info.java")
        && !modulePathDiagnostics.isEmpty()) {
      java.util.ArrayList<Diagnostic> combined =
          new java.util.ArrayList<>(result.diagnostics());
      combined.addAll(modulePathDiagnostics);
      result = new SemanticResult(result.packageName(), result.symbols(), combined);
    }
    analyses.put(fileName, new CachedAnalysis(source, result));
    return result;
  }

  synchronized EditorQueryResult editorQuery(String fileName, String source, int cursor) {
    if (moduleOverlay == null) {
      return EditorQueries.query(
          compiler, files, fileName, source, cursor, release, compilerOptions);
    }
    // An in-memory JavaFileObject has no module-oriented file-manager
    // location. javac 25 may therefore assert in Enter.moduleEnv before an
    // editor query can resolve even java.base symbols. Editor queries do not
    // publish JPMS diagnostics, so attribute them safely as an unnamed module
    // while retaining the complete dependency and source search paths.
    try (StandardJavaFileManager queryFiles =
        compiler.getStandardFileManager(null, Locale.ROOT, null)) {
      java.util.ArrayList<Path> queryClasspath = new java.util.ArrayList<>(classpath);
      queryClasspath.addAll(modulePath);
      if (!queryClasspath.isEmpty()) {
        queryFiles.setLocationFromPaths(
            StandardLocation.CLASS_PATH,
            queryClasspath.stream().filter(java.nio.file.Files::exists).toList());
      }
      queryFiles.setLocationFromPaths(
          StandardLocation.SOURCE_PATH, moduleQuerySourcePath);
      return EditorQueries.query(
          compiler, queryFiles, fileName, source, cursor, release, List.of());
    } catch (IOException failure) {
      throw new IllegalStateException("Unable to configure modular editor query", failure);
    }
  }

  synchronized boolean invalidate(String fileName) {
    return analyses.remove(fileName) != null;
  }

  synchronized int cachedDocumentCount() {
    return analyses.size();
  }

  synchronized long analysisCount() {
    return analysisCount;
  }

  @Override
  public synchronized void close() {
    try {
      files.close();
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to close semantic session", exception);
    }
    if (moduleOverlay != null) {
      deleteOverlay(moduleOverlay);
    }
    if (moduleQueryOverlay != null) {
      deleteOverlay(moduleQueryOverlay);
    }
  }

  private record CachedAnalysis(String source, SemanticResult result) {}

  private static void addModuleSource(
      Map<String, java.util.LinkedHashSet<String>> moduleSources, String mapping) {
    int separator = mapping.indexOf('=');
    if (separator <= 0 || separator == mapping.length() - 1) return;
    String module = mapping.substring(0, separator);
    java.util.LinkedHashSet<String> paths =
        moduleSources.computeIfAbsent(module, ignored -> new java.util.LinkedHashSet<>());
    for (String path : mapping.substring(separator + 1).split(java.util.regex.Pattern.quote(java.io.File.pathSeparator))) {
      if (!path.isBlank()) paths.add(path);
    }
  }

  private static void configureLocations(
      StandardJavaFileManager files, List<Path> classpath, List<Path> modulePath)
      throws IOException {
    if (!classpath.isEmpty()) {
      files.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath);
    }
    if (!modulePath.isEmpty()) {
      files.setLocationFromPaths(StandardLocation.MODULE_PATH, modulePath);
    }
  }

  private static List<Diagnostic> validateModulePath(List<Path> modulePath) {
    Map<String, Path> owners = new LinkedHashMap<>();
    List<Diagnostic> diagnostics = new java.util.ArrayList<>();
    for (Path entry : modulePath) {
      try {
        for (java.lang.module.ModuleReference module :
            java.lang.module.ModuleFinder.of(entry).findAll()) {
          Path previous = owners.putIfAbsent(module.descriptor().name(), entry);
          if (previous != null && !previous.equals(entry)) {
            diagnostics.add(
                new Diagnostic(
                    "error",
                    "jman.java.jpms.duplicate-module",
                    0,
                    0,
                    1,
                    1,
                    "duplicate module "
                        + module.descriptor().name()
                        + " found in "
                        + previous
                        + " and "
                        + entry));
          }
        }
      } catch (java.lang.module.FindException failure) {
        diagnostics.add(
            new Diagnostic(
                "error",
                "jman.java.jpms.invalid-module-path",
                0,
                0,
                1,
                1,
                failure.getMessage()));
      }
    }
    return List.copyOf(diagnostics);
  }

  private static void populateOverlay(Path overlay, List<Path> sourceRoots) {
    for (Path root : sourceRoots) {
      if (!java.nio.file.Files.isDirectory(root)) continue;
      try (var sources = java.nio.file.Files.walk(root)) {
        for (Path source :
            sources
                .filter(java.nio.file.Files::isRegularFile)
                .filter(path -> path.toString().endsWith(".java"))
                .toList()) {
          Path destination = overlay.resolve(root.relativize(source));
          if (java.nio.file.Files.exists(destination)) continue;
          java.nio.file.Files.createDirectories(destination.getParent());
          try {
            java.nio.file.Files.createSymbolicLink(destination, source);
          } catch (UnsupportedOperationException | IOException failure) {
            java.nio.file.Files.copy(source, destination);
          }
        }
      } catch (IOException exception) {
        throw new IllegalArgumentException("Unable to populate module overlay from " + root, exception);
      }
    }
  }

  private static List<Path> companionSources(
      Path overlay, Path primary, String packageName, String source) throws IOException {
    java.util.LinkedHashSet<Path> companions = new java.util.LinkedHashSet<>();
    Path packageDirectory =
        packageName.isEmpty() ? overlay : overlay.resolve(packageName.replace('.', '/'));
    if (java.nio.file.Files.isDirectory(packageDirectory)) {
      try (var siblings = java.nio.file.Files.list(packageDirectory)) {
        siblings
            .filter(java.nio.file.Files::isRegularFile)
            .filter(path -> path.toString().endsWith(".java"))
            .filter(path -> !path.equals(primary))
            .forEach(companions::add);
      }
    }
    java.util.regex.Matcher imports =
        java.util.regex.Pattern.compile("\\bimport\\s+(?:static\\s+)?([\\w.]+)(?:\\.\\*)?\\s*;")
            .matcher(source);
    while (imports.find()) {
      String imported = imports.group(1);
      while (imported.contains(".")) {
        Path candidate = overlay.resolve(imported.replace('.', '/') + ".java");
        if (java.nio.file.Files.isRegularFile(candidate)) {
          if (!candidate.equals(primary)) companions.add(candidate);
          break;
        }
        imported = imported.substring(0, imported.lastIndexOf('.'));
      }
    }
    return List.copyOf(companions);
  }

  private static void deleteOverlay(Path overlay) {
    try (var paths = java.nio.file.Files.walk(overlay)) {
      paths.sorted(java.util.Comparator.reverseOrder())
          .forEach(
              path -> {
                try {
                  java.nio.file.Files.deleteIfExists(path);
                } catch (IOException ignored) {
                  // Best-effort cleanup of ephemeral editor overlays.
                }
              });
    } catch (IOException ignored) {
      // Best-effort cleanup of ephemeral editor overlays.
    }
  }

  private static boolean causedByAssertion(Throwable failure) {
    for (Throwable cause = failure; cause != null; cause = cause.getCause()) {
      if (cause instanceof AssertionError) return true;
    }
    return false;
  }

}
