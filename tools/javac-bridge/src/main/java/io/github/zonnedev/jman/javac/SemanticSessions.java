package io.github.zonnedev.jman.javac;

import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

final class SemanticSessions {
  private static final AtomicLong NEXT_ID = new AtomicLong(1);
  private static final ConcurrentHashMap<Long, SemanticSession> SESSIONS =
      new ConcurrentHashMap<>();

  private SemanticSessions() {}

  static long create(
      List<Path> classpath,
      List<Path> modulePath,
      List<Path> sourcePath,
      Path moduleInfo,
      List<String> compilerOptions,
      int release) {
    long id = NEXT_ID.getAndIncrement();
    SESSIONS.put(
        id,
        new SemanticSession(
            classpath, modulePath, sourcePath, moduleInfo, compilerOptions, release));
    return id;
  }

  static SemanticResult analyze(long id, String fileName, String source) {
    SemanticSession session = SESSIONS.get(id);
    if (session == null) {
      throw new IllegalArgumentException("Unknown semantic session: " + id);
    }
    return session.analyze(fileName, source);
  }

  static EditorQueryResult editorQuery(long id, String fileName, String source, int cursor) {
    SemanticSession session = SESSIONS.get(id);
    if (session == null) {
      throw new IllegalArgumentException("Unknown semantic session: " + id);
    }
    return session.editorQuery(fileName, source, cursor);
  }

  static boolean destroy(long id) {
    SemanticSession session = SESSIONS.remove(id);
    if (session == null) {
      return false;
    }
    session.close();
    return true;
  }

  static boolean invalidate(long id, String fileName) {
    SemanticSession session = SESSIONS.get(id);
    if (session == null) {
      return false;
    }
    session.invalidate(fileName);
    return true;
  }

  static SemanticSession get(long id) {
    return SESSIONS.get(id);
  }
}
