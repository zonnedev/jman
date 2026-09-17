package io.github.zonnedev.jman.processor.worker;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Comparator;
import java.util.Properties;
import java.util.stream.Stream;

public final class ProcessorWorkerTest {
  private ProcessorWorkerTest() {}

  public static void main(String[] arguments) throws Exception {
    Path target = Path.of("/tmp/state/current/build/classes/java/integrationTest");
    Path staging = ProcessorWorker.stagingClassesDirectory(target);
    assert staging.equals(Path.of("/tmp/state/staging/build/classes/java/integrationTest"))
        : staging;
    Path partial = ProcessorWorker.partialClassesDirectory(target);
    assert partial.equals(Path.of("/tmp/state/partial/build/classes/java/integrationTest"))
        : partial;

    Path legacy = Path.of("/tmp/classes");
    assert ProcessorWorker.stagingClassesDirectory(legacy)
        .equals(Path.of("/tmp/classes.jman-java-staging"));
    assert ProcessorWorker.partialClassesDirectory(legacy)
        .equals(Path.of("/tmp/classes.jman-java-partial"));

    publishesUsefulProcessorOutputWhenAnotherSourceIsBroken();
  }

  private static void publishesUsefulProcessorOutputWhenAnotherSourceIsBroken() throws Exception {
    Path root = Files.createTempDirectory("jman-processor-partial");
    try {
      Path good = root.resolve("Good.java");
      Path broken = root.resolve("Broken.java");
      Path classes = root.resolve("state/current/classes");
      Path generated = root.resolve("generated");
      Path processorPath =
          Path.of(
              ProcessorWorkerTest.class
                  .getProtectionDomain()
                  .getCodeSource()
                  .getLocation()
                  .toURI());
      Files.writeString(good, "package fixture; public final class Good {}\n");
      Files.writeString(broken, "package fixture; public final class Broken { Missing value; }\n");
      Properties request = new Properties();
      request.setProperty("protocol.version", Integer.toString(ProcessorWorker.PROTOCOL_VERSION));
      request.setProperty("release", "17");
      request.setProperty("generated.directory", generated.toString());
      request.setProperty("classes.directory", classes.toString());
      request.setProperty("source.count", "2");
      request.setProperty("source.0", good.toString());
      request.setProperty("source.1", broken.toString());
      request.setProperty("source.path.count", "0");
      request.setProperty("classpath.count", "1");
      request.setProperty("classpath.0", processorPath.toString());
      request.setProperty("processor.path.count", "1");
      request.setProperty("processor.path.0", processorPath.toString());
      request.setProperty("processor.option.count", "2");
      request.setProperty("processor.option.0", "-processor");
      request.setProperty("processor.option.1", PartialOutputProcessor.class.getName());
      Path requestFile = root.resolve("request.properties");
      try (var output = Files.newOutputStream(requestFile)) {
        request.store(output, null);
      }

      boolean failed = false;
      try {
        ProcessorWorker.process(requestFile);
      } catch (IllegalStateException expected) {
        failed = true;
      }

      assert failed : "the broken source unexpectedly compiled";
      Path partial = ProcessorWorker.partialClassesDirectory(classes);
      assert Files.isRegularFile(partial.resolve("partial-marker.txt")) : partial;
      assert !Files.exists(classes) : classes;
    } finally {
      try (Stream<Path> paths = Files.walk(root)) {
        for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
          Files.delete(path);
        }
      }
    }
  }
}
