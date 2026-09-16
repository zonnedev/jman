package io.github.zonnedev.jman.maven.importer;

import java.util.List;

record MavenCompileModel(
    int schemaVersion,
    String buildSystem,
    String projectPath,
    String projectDirectory,
    String taskPath,
    List<String> sourceFiles,
    List<String> sourceRoots,
    List<String> classpath,
    List<String> modulePath,
    List<String> projectDependencies,
    List<String> annotationProcessorPath,
    List<String> annotationProcessorOptions,
    List<String> compilerArgs,
    String release,
    String encoding,
    String generatedSourcesDirectory,
    List<String> generatedSourceDirectories,
    String destinationDirectory,
    String javaCompilerExecutable,
    String javaLanguageVersion) {}
