package io.github.zonnedev.jman.tests.fixture;

import java.io.IOException;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.AbstractProcessor;
import javax.annotation.processing.RoundEnvironment;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
import javax.tools.JavaFileObject;

public final class GreetingProcessor extends AbstractProcessor {
  @Override
  public Set<String> getSupportedAnnotationTypes() {
    return Set.of(GenerateGreeting.class.getCanonicalName());
  }

  @Override
  public SourceVersion getSupportedSourceVersion() {
    return SourceVersion.RELEASE_25;
  }

  @Override
  public Set<String> getSupportedOptions() {
    return Set.of("greeting.mode");
  }

  @Override
  public boolean process(
      Set<? extends TypeElement> annotations, RoundEnvironment roundEnvironment) {
    if (annotations.isEmpty()) {
      return false;
    }

    try {
      JavaFileObject generated =
          processingEnv.getFiler().createSourceFile("io.github.zonnedev.jman.tests.fixture.GeneratedGreeting");
      try (Writer writer = generated.openWriter()) {
        writer.write(
            """
            package io.github.zonnedev.jman.tests.fixture;

            public final class GeneratedGreeting {
              private GeneratedGreeting() {}

              public static String message() {
                return "generated";
              }
            }
            """);
      }
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to generate greeting", exception);
    }
    return true;
  }
}
