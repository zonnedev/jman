package io.github.zonnedev.jman.maven.importer;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilderFactory;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.Node;

public final class MavenModelImporter {
  private MavenModelImporter() {}

  public static void main(String[] arguments) throws Exception {
    if (arguments.length != 3) {
      throw new IllegalArgumentException(
          "usage: MavenModelImporter EFFECTIVE_POM CLASSPATH_FILE LOCAL_REPOSITORY");
    }
    Path effectivePom = Path.of(arguments[0]).toAbsolutePath().normalize();
    Path classpathFile = Path.of(arguments[1]).toAbsolutePath().normalize();
    Path localRepository = Path.of(arguments[2]).toAbsolutePath().normalize();
    for (MavenCompileModel model : importModels(effectivePom, classpathFile, localRepository)) {
      System.out.println(JsonWriter.write(model));
    }
  }

  static List<MavenCompileModel> importModels(
      Path effectivePom, Path classpathFile, Path localRepository) throws Exception {
    DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
    factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
    factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
    factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
    factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "");
    factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "");
    Document document = factory.newDocumentBuilder().parse(effectivePom.toFile());
    Element root = document.getDocumentElement();
    List<Element> projects;
    if (hasName(root, "project")) {
      projects = List.of(root);
    } else if (hasName(root, "projects")) {
      projects = children(root, "project");
    } else {
      throw new IllegalArgumentException("effective POM root must be <project> or <projects>");
    }

    List<String> reactorArtifacts =
        projects.stream().map(project -> text(project, "artifactId")).filter(value -> value != null).toList();
    List<MavenCompileModel> models = new ArrayList<>();
    for (Element project : projects) {
      Path projectDirectory = projectDirectory(project, effectivePom);
      Path moduleClasspath = projectDirectory.resolve("target/javac-frontend-classpath.txt");
      models.addAll(
          importProject(
              project,
              Files.isRegularFile(moduleClasspath) ? moduleClasspath : classpathFile,
              localRepository,
              projectDirectory,
              reactorArtifacts));
    }
    return List.copyOf(models);
  }

  private static List<MavenCompileModel> importProject(
      Element project,
      Path classpathFile,
      Path localRepository,
      Path projectDirectory,
      List<String> reactorArtifacts)
      throws IOException {
    List<String> dependencyClasspath = readClasspath(classpathFile);
    Element compiler = compilerConfiguration(project);
    List<String> processorPath = annotationProcessorPath(compiler, localRepository);
    List<String> compilerArgs = compilerArguments(compiler);
    List<String> processorOptions = processorArguments(compilerArgs);
    List<String> configuredModulePath = optionPath(compilerArgs, "--module-path");
    List<String> projectDependencies = projectDependencies(project, reactorArtifacts);
    String encoding = firstNonBlank(text(compiler, "encoding"), property(project, "project.build.sourceEncoding"));
    String mainRelease = firstNonBlank(text(compiler, "release"), property(project, "maven.compiler.release"));
    String testRelease =
        firstNonBlank(
            text(compiler, "testRelease"),
            property(project, "maven.compiler.testRelease"),
            mainRelease);
    String artifactId = text(project, "artifactId");
    String sourceDirectory =
        firstNonBlank(text(child(project, "build"), "sourceDirectory"), projectDirectory.resolve("src/main/java").toString());
    String testSourceDirectory =
        firstNonBlank(text(child(project, "build"), "testSourceDirectory"), projectDirectory.resolve("src/test/java").toString());
    String outputDirectory =
        firstNonBlank(text(child(project, "build"), "outputDirectory"), projectDirectory.resolve("target/classes").toString());
    String testOutputDirectory =
        firstNonBlank(
            text(child(project, "build"), "testOutputDirectory"),
            projectDirectory.resolve("target/test-classes").toString());
    boolean modularProject = Files.isRegularFile(Path.of(sourceDirectory).resolve("module-info.java"));
    List<String> modulePath = new ArrayList<>(configuredModulePath);
    List<String> mainClasspath = new ArrayList<>();
    for (String entry : dependencyClasspath) {
      if (modularProject && isModulePathEntry(Path.of(entry))) {
        if (!modulePath.contains(entry)) modulePath.add(entry);
      } else {
        mainClasspath.add(entry);
      }
    }

    MavenCompileModel main =
        model(
            artifactId,
            projectDirectory,
            "compile",
            sourceDirectory,
            mainClasspath,
            modulePath,
            projectDependencies,
            processorPath,
            processorOptions,
            compilerArgs,
            mainRelease,
            encoding,
            projectDirectory.resolve("target/generated-sources/annotations").toString(),
            outputDirectory);
    List<String> testClasspath = new ArrayList<>();
    List<String> testModulePath = new ArrayList<>(modulePath);
    if (modularProject) {
      testModulePath.add(outputDirectory);
    } else {
      testClasspath.add(outputDirectory);
    }
    testClasspath.addAll(mainClasspath);
    MavenCompileModel test =
        model(
            artifactId,
            projectDirectory,
            "testCompile",
            testSourceDirectory,
            testClasspath,
            testModulePath,
            projectDependencies,
            processorPath,
            processorOptions,
            compilerArgs,
            testRelease,
            encoding,
            projectDirectory.resolve("target/generated-test-sources/test-annotations").toString(),
            testOutputDirectory);
    return List.of(main, test);
  }

  private static boolean isModulePathEntry(Path entry) {
    if (entry.toString().endsWith(".jar")) return true;
    return Files.isDirectory(entry) && Files.isRegularFile(entry.resolve("module-info.class"));
  }

  private static MavenCompileModel model(
      String artifactId,
      Path projectDirectory,
      String task,
      String sourceDirectory,
      List<String> classpath,
      List<String> modulePath,
      List<String> projectDependencies,
      List<String> processorPath,
      List<String> processorOptions,
      List<String> compilerArgs,
      String release,
      String encoding,
      String generatedSources,
      String destination) {
    return new MavenCompileModel(
        2,
        "maven",
        artifactId,
        projectDirectory.toString(),
        task,
        javaSources(Path.of(sourceDirectory)),
        List.of(Path.of(sourceDirectory).toAbsolutePath().normalize().toString()),
        List.copyOf(classpath),
        List.copyOf(modulePath),
        List.copyOf(projectDependencies),
        List.copyOf(processorPath),
        List.copyOf(processorOptions),
        List.copyOf(compilerArgs),
        release,
        encoding,
        generatedSources,
        generatedSourceDirectories(projectDirectory, generatedSources),
        destination,
        null,
        release);
  }

  private static List<String> generatedSourceDirectories(
      Path projectDirectory, String defaultDirectory) {
    List<String> result = new ArrayList<>();
    Path generatedRoot = projectDirectory.resolve("target/generated-sources");
    if (Files.isDirectory(generatedRoot)) {
      try (var entries = Files.list(generatedRoot)) {
        entries
            .filter(Files::isDirectory)
            .map(path -> path.toAbsolutePath().normalize().toString())
            .sorted()
            .forEach(result::add);
      } catch (IOException exception) {
        throw new IllegalStateException("cannot enumerate generated roots " + generatedRoot, exception);
      }
    }
    String normalizedDefault = Path.of(defaultDirectory).toAbsolutePath().normalize().toString();
    if (!result.contains(normalizedDefault)) {
      result.add(normalizedDefault);
    }
    return List.copyOf(result);
  }

  private static List<String> optionPath(List<String> arguments, String option) {
    for (int index = 0; index < arguments.size(); index++) {
      String argument = arguments.get(index);
      String encoded =
          argument.equals(option) && index + 1 < arguments.size()
              ? arguments.get(index + 1)
              : argument.startsWith(option + "=") ? argument.substring(option.length() + 1) : null;
      if (encoded != null) {
        return Arrays.stream(encoded.split(java.util.regex.Pattern.quote(System.getProperty("path.separator"))))
            .filter(value -> !value.isBlank())
            .map(value -> Path.of(value).toAbsolutePath().normalize().toString())
            .toList();
      }
    }
    return List.of();
  }

  private static List<String> projectDependencies(
      Element project, List<String> reactorArtifacts) {
    List<String> result = new ArrayList<>();
    for (Element dependency : children(project, "dependency")) {
      String artifactId = text(dependency, "artifactId");
      if (artifactId != null && reactorArtifacts.contains(artifactId) && !result.contains(artifactId)) {
        result.add(artifactId);
      }
    }
    for (Element dependency : children(child(project, "dependencies"), "dependency")) {
      String artifactId = text(dependency, "artifactId");
      if (artifactId != null && reactorArtifacts.contains(artifactId) && !result.contains(artifactId)) {
        result.add(artifactId);
      }
    }
    return List.copyOf(result);
  }

  private static List<String> javaSources(Path root) {
    if (!Files.isDirectory(root)) {
      return List.of();
    }
    try (var paths = Files.walk(root)) {
      return paths
          .filter(path -> path.toString().endsWith(".java"))
          .map(path -> path.toAbsolutePath().normalize().toString())
          .sorted()
          .toList();
    } catch (IOException exception) {
      throw new IllegalStateException("cannot enumerate source root " + root, exception);
    }
  }

  private static Path projectDirectory(Element project, Path effectivePom) {
    String sourceDirectory = text(child(project, "build"), "sourceDirectory");
    if (sourceDirectory != null) {
      Path source = Path.of(sourceDirectory).toAbsolutePath().normalize();
      Path suffix = Path.of("src/main/java");
      if (source.endsWith(suffix)) {
        return source.getParent().getParent().getParent();
      }
    }
    Path target = effectivePom.getParent();
    return target != null && target.getFileName().toString().equals("target")
        ? target.getParent()
        : effectivePom.getParent();
  }

  private static List<String> readClasspath(Path classpathFile) throws IOException {
    if (!Files.isRegularFile(classpathFile)) {
      return List.of();
    }
    String content = Files.readString(classpathFile).trim();
    if (content.isEmpty()) {
      return List.of();
    }
    return Arrays.stream(content.split(java.util.regex.Pattern.quote(System.getProperty("path.separator"))))
        .filter(value -> !value.isBlank())
        .map(value -> Path.of(value).toAbsolutePath().normalize().toString())
        .distinct()
        .toList();
  }

  private static Element compilerConfiguration(Element project) {
    Element build = child(project, "build");
    Element direct = compilerConfigurationIn(child(build, "plugins"));
    if (direct != null) {
      return direct;
    }
    return compilerConfigurationIn(child(child(build, "pluginManagement"), "plugins"));
  }

  private static Element compilerConfigurationIn(Element plugins) {
    for (Element plugin : children(plugins, "plugin")) {
      if ("maven-compiler-plugin".equals(text(plugin, "artifactId"))) {
        return child(plugin, "configuration");
      }
    }
    return null;
  }

  private static List<String> compilerArguments(Element configuration) {
    Element arguments = child(configuration, "compilerArgs");
    List<String> result = new ArrayList<>();
    if (arguments != null) {
      for (Node node = arguments.getFirstChild(); node != null; node = node.getNextSibling()) {
        if (node instanceof Element element
            && (hasName(element, "arg") || hasName(element, "compilerArg"))) {
          result.add(element.getTextContent().trim());
        }
      }
    }
    Element processors = child(configuration, "annotationProcessors");
    List<String> processorNames =
        children(processors, "annotationProcessor").stream()
            .map(Element::getTextContent)
            .map(String::trim)
            .filter(value -> !value.isEmpty())
            .toList();
    if (!processorNames.isEmpty()) {
      result.add("-processor");
      result.add(String.join(",", processorNames));
    }
    return List.copyOf(result);
  }

  private static List<String> processorArguments(List<String> compilerArguments) {
    List<String> result = new ArrayList<>();
    for (int index = 0; index < compilerArguments.size(); index++) {
      String argument = compilerArguments.get(index);
      if (argument.startsWith("-A") || argument.startsWith("-processor=")) {
        result.add(argument);
      } else if (argument.equals("-processor") && index + 1 < compilerArguments.size()) {
        result.add(argument);
        result.add(compilerArguments.get(++index));
      }
    }
    return List.copyOf(result);
  }

  private static List<String> annotationProcessorPath(Element configuration, Path repository) {
    Element paths = child(configuration, "annotationProcessorPaths");
    List<String> result = new ArrayList<>();
    for (Element path : children(paths, "path")) {
      String groupId = text(path, "groupId");
      String artifactId = text(path, "artifactId");
      String version = text(path, "version");
      if (groupId == null || artifactId == null || version == null) {
        continue;
      }
      String classifier = text(path, "classifier");
      String extension = firstNonBlank(text(path, "type"), "jar");
      String fileName =
          artifactId
              + "-"
              + version
              + (classifier == null ? "" : "-" + classifier)
              + "."
              + extension;
      result.add(
          repository
              .resolve(groupId.replace('.', '/'))
              .resolve(artifactId)
              .resolve(version)
              .resolve(fileName)
              .toString());
    }
    return List.copyOf(result);
  }

  private static String property(Element project, String name) {
    return text(child(project, "properties"), name);
  }

  private static Element child(Element parent, String name) {
    if (parent == null) {
      return null;
    }
    for (Node node = parent.getFirstChild(); node != null; node = node.getNextSibling()) {
      if (node instanceof Element element && hasName(element, name)) {
        return element;
      }
    }
    return null;
  }

  private static List<Element> children(Element parent, String name) {
    if (parent == null) {
      return List.of();
    }
    List<Element> result = new ArrayList<>();
    for (Node node = parent.getFirstChild(); node != null; node = node.getNextSibling()) {
      if (node instanceof Element element && hasName(element, name)) {
        result.add(element);
      }
    }
    return result;
  }

  private static String text(Element parent, String name) {
    Element element = child(parent, name);
    if (element == null) {
      return null;
    }
    String value = element.getTextContent().trim();
    return value.isEmpty() ? null : value;
  }

  private static boolean hasName(Element element, String name) {
    return name.equals(element.getLocalName()) || name.equals(element.getNodeName());
  }

  private static String firstNonBlank(String... values) {
    for (String value : values) {
      if (value != null && !value.isBlank()) {
        return value;
      }
    }
    return null;
  }
}
