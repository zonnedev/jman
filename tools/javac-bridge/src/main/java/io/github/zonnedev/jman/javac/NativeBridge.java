package io.github.zonnedev.jman.javac;

import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.List;
import org.graalvm.nativeimage.IsolateThread;
import org.graalvm.nativeimage.UnmanagedMemory;
import org.graalvm.nativeimage.c.function.CEntryPoint;
import org.graalvm.nativeimage.c.type.CCharPointer;
import org.graalvm.word.UnsignedWord;
import org.graalvm.word.WordFactory;

public final class NativeBridge {
  private static final int ABI_VERSION = 3;

  private NativeBridge() {}

  public static void main(String[] args) {
    // Native Image 25.0.4 expects a main method when a class is supplied to a
    // shared-library build, even though consumers use CEntryPoint methods.
  }

  @CEntryPoint(name = "javac_frontend_abi_version")
  static int abiVersion(IsolateThread thread) {
    return ABI_VERSION;
  }

  @CEntryPoint(name = "javac_frontend_parse")
  static CCharPointer parse(
      IsolateThread thread, CCharPointer sourcePointer, UnsignedWord sourceLength) {
    ensureJavaHome();
    String source = readUtf8(sourcePointer, sourceLength);
    byte[] payload = WireEncoder.encode(JavacFrontend.parse("Input.java", source));
    return allocate(payload);
  }

  @CEntryPoint(name = "javac_frontend_analyze")
  static CCharPointer analyze(
      IsolateThread thread,
      CCharPointer sourcePointer,
      UnsignedWord sourceLength,
      CCharPointer fileNamePointer,
      UnsignedWord fileNameLength,
      CCharPointer classpathPointer,
      UnsignedWord classpathLength,
      CCharPointer sourcePathPointer,
      UnsignedWord sourcePathLength,
      int release) {
    ensureJavaHome();
    String source = readUtf8(sourcePointer, sourceLength);
    String fileName = readUtf8(fileNamePointer, fileNameLength);
    String encodedClasspath = readUtf8(classpathPointer, classpathLength);
    List<Path> classpath = decodePathList(encodedClasspath);
    List<Path> sourcePath = decodePathList(readUtf8(sourcePathPointer, sourcePathLength));
    byte[] payload =
        WireEncoder.encode(JavacFrontend.analyze(fileName, source, classpath, sourcePath, release));
    return allocate(payload);
  }

  @CEntryPoint(name = "javac_frontend_workspace_parse")
  static CCharPointer parseWorkspace(
      IsolateThread thread,
      CCharPointer inputPointer,
      UnsignedWord inputLength,
      int release,
      int enablePreview) {
    ensureJavaHome();
    try {
      byte[] encoded = readBytes(inputPointer, inputLength);
      BatchInput input = new BatchInput(encoded);
      int count = input.readInt();
      if (count < 0 || count > 1_000_000) {
        throw new IllegalArgumentException("invalid workspace source count");
      }
      List<SourceInput> sources = new java.util.ArrayList<>(count);
      for (int index = 0; index < count; index++) {
        sources.add(new SourceInput(input.readString(), input.readString()));
      }
      input.requireEnd();
      return allocate(
          WireEncoder.encode(
              JavacFrontend.parseWorkspace(sources, release, enablePreview != 0)));
    } catch (Throwable failure) {
      return WordFactory.nullPointer();
    }
  }

  @CEntryPoint(name = "javac_frontend_session_create")
  static long createSession(
      IsolateThread thread,
      CCharPointer classpathPointer,
      UnsignedWord classpathLength,
      CCharPointer sourcePathPointer,
      UnsignedWord sourcePathLength,
      CCharPointer modulePathPointer,
      UnsignedWord modulePathLength,
      CCharPointer moduleInfoPointer,
      UnsignedWord moduleInfoLength,
      CCharPointer compilerOptionsPointer,
      UnsignedWord compilerOptionsLength,
      int release) {
    ensureJavaHome();
    String moduleInfo = readUtf8(moduleInfoPointer, moduleInfoLength);
    return SemanticSessions.create(
        decodePathList(readUtf8(classpathPointer, classpathLength)),
        decodePathList(readUtf8(modulePathPointer, modulePathLength)),
        decodePathList(readUtf8(sourcePathPointer, sourcePathLength)),
        moduleInfo.isBlank() ? null : Path.of(moduleInfo),
        decodeStringList(readUtf8(compilerOptionsPointer, compilerOptionsLength)),
        release);
  }

  @CEntryPoint(name = "javac_frontend_session_analyze")
  static CCharPointer analyzeSession(
      IsolateThread thread,
      long sessionId,
      CCharPointer sourcePointer,
      UnsignedWord sourceLength,
      CCharPointer fileNamePointer,
      UnsignedWord fileNameLength) {
    ensureJavaHome();
    try {
      byte[] payload =
          WireEncoder.encode(
              SemanticSessions.analyze(
                  sessionId,
                  readUtf8(fileNamePointer, fileNameLength),
                  readUtf8(sourcePointer, sourceLength)));
      return allocate(payload);
    } catch (Throwable failure) {
      // No Java exception may cross a Graal C entry point: Native Image treats
      // that as a fatal process error. A null result is converted into a
      // recoverable FrontendError by the Rust facade.
      failure.printStackTrace(System.err);
      return WordFactory.nullPointer();
    }
  }

  @CEntryPoint(name = "javac_frontend_session_editor_query")
  static CCharPointer editorQuerySession(
      IsolateThread thread,
      long sessionId,
      CCharPointer sourcePointer,
      UnsignedWord sourceLength,
      CCharPointer fileNamePointer,
      UnsignedWord fileNameLength,
      int cursor) {
    ensureJavaHome();
    try {
      return allocate(
          WireEncoder.encode(
              SemanticSessions.editorQuery(
                  sessionId,
                  readUtf8(fileNamePointer, fileNameLength),
                  readUtf8(sourcePointer, sourceLength),
                  cursor)));
    } catch (Throwable failure) {
      failure.printStackTrace(System.err);
      return WordFactory.nullPointer();
    }
  }

  @CEntryPoint(name = "javac_frontend_session_destroy")
  static int destroySession(IsolateThread thread, long sessionId) {
    return SemanticSessions.destroy(sessionId) ? 0 : 1;
  }

  @CEntryPoint(name = "javac_frontend_session_invalidate")
  static int invalidateSession(
      IsolateThread thread,
      long sessionId,
      CCharPointer fileNamePointer,
      UnsignedWord fileNameLength) {
    return SemanticSessions.invalidate(
            sessionId, readUtf8(fileNamePointer, fileNameLength))
        ? 0
        : 1;
  }

  private static CCharPointer allocate(byte[] payload) {
    CCharPointer allocation =
        UnmanagedMemory.malloc(WordFactory.unsigned(Integer.BYTES + payload.length));
    if (allocation.isNull()) {
      return WordFactory.nullPointer();
    }
    writeLittleEndianInt(allocation, payload.length);
    for (int index = 0; index < payload.length; index++) {
      allocation.write(Integer.BYTES + index, payload[index]);
    }
    return allocation;
  }

  private static String readUtf8(CCharPointer pointer, UnsignedWord encodedLength) {
    return new String(readBytes(pointer, encodedLength), StandardCharsets.UTF_8);
  }

  private static byte[] readBytes(CCharPointer pointer, UnsignedWord encodedLength) {
    int length = Math.toIntExact(encodedLength.rawValue());
    byte[] bytes = new byte[length];
    for (int index = 0; index < length; index++) {
      bytes[index] = pointer.read(index);
    }
    return bytes;
  }

  private static List<Path> decodePathList(String encodedPaths) {
    return encodedPaths.isEmpty()
        ? List.of()
        : Arrays.stream(encodedPaths.split(java.io.File.pathSeparator, -1))
            .filter(entry -> !entry.isBlank())
            .map(Path::of)
            .toList();
  }

  private static List<String> decodeStringList(String encoded) {
    return encoded.isEmpty() ? List.of() : Arrays.asList(encoded.split("\\u0000", -1));
  }

  @CEntryPoint(name = "javac_frontend_free")
  static void free(IsolateThread thread, CCharPointer result) {
    UnmanagedMemory.free(result);
  }

  private static void writeLittleEndianInt(CCharPointer output, int value) {
    output.write(0, (byte) value);
    output.write(1, (byte) (value >>> 8));
    output.write(2, (byte) (value >>> 16));
    output.write(3, (byte) (value >>> 24));
  }

  private static void ensureJavaHome() {
    if (System.getProperty("java.home") != null) {
      return;
    }
    String javaHome = System.getenv("JAVA_HOME");
    if (javaHome == null || javaHome.isBlank()) {
      throw new IllegalStateException(
          "java.home is unavailable; set JAVA_HOME to the project JDK before creating the frontend");
    }
    System.setProperty("java.home", javaHome);
  }

  private static final class BatchInput {
    private final byte[] bytes;
    private int offset;

    BatchInput(byte[] bytes) {
      this.bytes = bytes;
    }

    int readInt() {
      if (offset + Integer.BYTES > bytes.length) {
        throw new IllegalArgumentException("truncated batch input");
      }
      int value =
          ((bytes[offset] & 0xff) << 24)
              | ((bytes[offset + 1] & 0xff) << 16)
              | ((bytes[offset + 2] & 0xff) << 8)
              | (bytes[offset + 3] & 0xff);
      offset += Integer.BYTES;
      return value;
    }

    String readString() {
      int length = readInt();
      if (length < 0 || offset + length > bytes.length) {
        throw new IllegalArgumentException("invalid batch string");
      }
      String value = new String(bytes, offset, length, StandardCharsets.UTF_8);
      offset += length;
      return value;
    }

    void requireEnd() {
      if (offset != bytes.length) {
        throw new IllegalArgumentException("trailing batch input");
      }
    }
  }
}
