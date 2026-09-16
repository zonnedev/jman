package io.github.zonnedev.jman.processor.worker;

import java.nio.file.Path;

public final class ProcessorWorkerTest {
  private ProcessorWorkerTest() {}

  public static void main(String[] arguments) {
    Path target = Path.of("/tmp/state/current/build/classes/java/integrationTest");
    Path staging = ProcessorWorker.stagingClassesDirectory(target);
    assert staging.equals(Path.of("/tmp/state/staging/build/classes/java/integrationTest"))
        : staging;

    Path legacy = Path.of("/tmp/classes");
    assert ProcessorWorker.stagingClassesDirectory(legacy)
        .equals(Path.of("/tmp/classes.jman-java-staging"));
  }
}
