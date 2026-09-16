package io.github.zonnedev.jman.javac;

import java.net.URI;
import javax.tools.SimpleJavaFileObject;

final class StringJavaFileObject extends SimpleJavaFileObject {
  private final String source;
  private final String logicalName;

  StringJavaFileObject(String name, String source) {
    super(URI.create("mem:///" + name), Kind.SOURCE);
    logicalName = name;
    this.source = source;
  }

  String logicalName() {
    return logicalName;
  }

  @Override
  public CharSequence getCharContent(boolean ignoreEncodingErrors) {
    return source;
  }
}
