package io.github.zonnedev.jman.processor.worker;

import java.io.IOException;
import java.io.Writer;
import java.util.Set;
import javax.annotation.processing.AbstractProcessor;
import javax.annotation.processing.RoundEnvironment;
import javax.lang.model.SourceVersion;
import javax.lang.model.element.TypeElement;
import javax.tools.StandardLocation;

public final class PartialOutputProcessor extends AbstractProcessor {
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
  public boolean process(Set<? extends TypeElement> annotations, RoundEnvironment roundEnvironment) {
    if (!generated && !roundEnvironment.processingOver()) {
      generated = true;
      try (Writer marker =
          processingEnv
              .getFiler()
              .createResource(StandardLocation.CLASS_OUTPUT, "", "partial-marker.txt")
              .openWriter()) {
        marker.write("generated before a later compilation error\n");
      } catch (IOException error) {
        throw new IllegalStateException(error);
      }
    }
    return false;
  }
}
