package io.github.zonnedev.jman.maven.importer;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

final class MavenModelImporterTest {
  public static void main(String[] arguments) throws Exception {
    importsCompilerAndAnnotationProcessorConfiguration();
  }

  private static void importsCompilerAndAnnotationProcessorConfiguration() throws Exception {
    Path directory = Files.createTempDirectory("maven-model-test");
    Path sources = Files.createDirectories(directory.resolve("src/main/java/demo"));
    Files.writeString(sources.resolve("Example.java"), "package demo; class Example {}");
    Path effectivePom = Files.createDirectories(directory.resolve("target")).resolve("effective.xml");
    Files.writeString(
        effectivePom,
        """
        <project>
          <artifactId>example</artifactId>
          <properties>
            <maven.compiler.release>25</maven.compiler.release>
            <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>
          </properties>
          <build>
            <sourceDirectory>%s</sourceDirectory>
            <plugins><plugin>
              <artifactId>maven-compiler-plugin</artifactId>
              <configuration>
                <compilerArgs>
                  <arg>-parameters</arg><arg>-Xlint:all</arg>
                  <arg>-Ademo.mode=strict</arg>
                  <arg>--module-path=%s</arg>
                </compilerArgs>
                <annotationProcessorPaths><path>
                  <groupId>demo.processor</groupId><artifactId>generator</artifactId><version>1.2</version>
                </path></annotationProcessorPaths>
                <annotationProcessors>
                  <annotationProcessor>demo.processor.Generator</annotationProcessor>
                </annotationProcessors>
              </configuration>
            </plugin></plugins>
          </build>
        </project>
        """
            .formatted(
                directory.resolve("src/main/java"),
                directory.resolve("modules")));
    Path classpath = directory.resolve("classpath.txt");
    Files.writeString(classpath, directory.resolve("dependency.jar").toString());
    Path repository = directory.resolve("repository");

    List<MavenCompileModel> models =
        MavenModelImporter.importModels(effectivePom, classpath, repository);
    MavenCompileModel main = models.getFirst();

    assert main.schemaVersion() == 2;
    assert main.buildSystem().equals("maven");
    assert main.projectPath().equals("example");
    assert main.release().equals("25");
    assert main.encoding().equals("UTF-8");
    assert main.compilerArgs().contains("-parameters");
    assert main.annotationProcessorOptions().equals(
        List.of("-Ademo.mode=strict", "-processor", "demo.processor.Generator"));
    assert main.modulePath().equals(List.of(directory.resolve("modules").toString()));
    assert main.sourceFiles().size() == 1;
    assert main.sourceRoots().equals(List.of(directory.resolve("src/main/java").toString()));
    assert !main.generatedSourceDirectories().isEmpty();
    assert main.annotationProcessorPath().getFirst().endsWith("generator-1.2.jar");
    assert JsonWriter.write(main).contains("\"annotationProcessorPath\"");
    assert JsonWriter.write(main).contains("\"schemaVersion\":2");
  }
}
