package io.github.zonnedev.jman.javac;

import com.sun.source.doctree.DocCommentTree;
import com.sun.source.tree.ClassTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.DocTrees;
import com.sun.source.util.JavacTask;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.tools.JavaCompiler;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;

final class JavadocSourceIndex {
  private static final ConcurrentHashMap<String, Map<String, String>> CACHE =
      new ConcurrentHashMap<>();

  private JavadocSourceIndex() {}

  static String lookup(
      JavaCompiler compiler, StandardJavaFileManager files, Element element, int release) {
    TypeElement owner = owner(element);
    if (owner == null) return "";
    String relative = owner.getQualifiedName().toString()
        .replace('.', '/')
        .replaceAll("\\$.*$", "") + ".java";
    try {
      for (Path root : files.getLocationAsPaths(StandardLocation.SOURCE_PATH)) {
        Source source = findSource(root, relative, release);
        if (source == null) continue;
        Map<String, String> documentation =
            CACHE.computeIfAbsent(
                source.cacheKey(),
                ignored -> parse(compiler, source.fileName(), source.content()));
        String value = documentation.get(key(element));
        if (value != null && !value.isBlank()) return value;
      }
    } catch (RuntimeException ignored) {
      // Documentation is optional and must never break an editor query.
    }
    return "";
  }

  static EditorDefinition definition(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      JavacTask semanticTask,
      Element element,
      int release) {
    TypeElement owner = owner(element);
    if (owner == null) return null;
    String relative =
        owner.getQualifiedName().toString().replace('.', '/').replaceAll("\\$.*$", "")
            + ".java";
    try {
      for (Path root : files.getLocationAsPaths(StandardLocation.SOURCE_PATH)) {
        Source source = findSource(root, relative, release);
        if (source == null) continue;
        long[] range = declarationRange(compiler, source.fileName(), source.content(), element);
        if (range != null) {
          return new EditorDefinition(
              SymbolIds.of(semanticTask.getElements(), semanticTask.getTypes(), element),
              SymbolIds.moduleName(semanticTask.getElements(), element),
              SymbolIds.binaryOwner(semanticTask.getElements(), element),
              element.getSimpleName().toString(),
              SymbolIds.descriptor(semanticTask.getElements(), semanticTask.getTypes(), element),
              source.fileName(),
              source.content(),
              range[0],
              range[1],
              false);
        }
      }
    } catch (RuntimeException ignored) {
      // Source navigation is optional and decompilation may still handle it.
    }
    return null;
  }

  private static long[] declarationRange(
      JavaCompiler compiler, String fileName, String source, Element target) {
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                null,
                null,
                List.of("-proc:none"),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      Trees trees = Trees.instance(task);
      long[][] result = {null};
      new TreePathScanner<Void, Void>() {
        @Override
        public Void scan(Tree tree, Void unused) {
          if (result[0] == null && matches(tree, target)) {
            long start = trees.getSourcePositions().getStartPosition(unit, tree);
            long end = trees.getSourcePositions().getEndPosition(unit, tree);
            String name = sourceName(target);
            int identifier = source.indexOf(name, Math.max(0, Math.toIntExact(start)));
            if (identifier >= 0 && identifier + name.length() <= end) {
              result[0] = new long[] {identifier, identifier + name.length()};
            }
          }
          return super.scan(tree, unused);
        }
      }.scan(unit, null);
      return result[0];
    } catch (IOException | RuntimeException ignored) {
      return null;
    }
  }

  private static String sourceName(Element element) {
    if (element.getKind() == ElementKind.CONSTRUCTOR
        && element.getEnclosingElement() instanceof TypeElement owner) {
      return owner.getSimpleName().toString();
    }
    return element.getSimpleName().toString();
  }

  private static boolean matches(Tree tree, Element target) {
    if (tree instanceof MethodTree method && target instanceof ExecutableElement executable) {
      if (!method.getName().contentEquals(executable.getSimpleName())
          || method.getParameters().size() != executable.getParameters().size()) {
        return false;
      }
      for (int index = 0; index < method.getParameters().size(); index++) {
        String sourceType = normalizedType(method.getParameters().get(index).getType().toString());
        String resolvedType =
            normalizedType(executable.getParameters().get(index).asType().toString());
        if (!sourceType.equals(resolvedType)) return false;
      }
      return true;
    }
    if (tree instanceof VariableTree variable && target instanceof VariableElement field) {
      return variable.getName().contentEquals(field.getSimpleName());
    }
    if (tree instanceof ClassTree type && target instanceof TypeElement element) {
      return type.getSimpleName().contentEquals(element.getSimpleName());
    }
    return false;
  }

  private static String normalizedType(String type) {
    String withoutGenerics = type.replaceAll("<[^<>]*(?:<[^<>]*>[^<>]*)*>", "");
    StringBuilder result = new StringBuilder();
    for (String token : withoutGenerics.replace("...", "[]").split("(?=[\\[\\], ?&])|(?<=[\\[\\], ?&])")) {
      String trimmed = token.trim();
      if (trimmed.isEmpty()
          || trimmed.equals("?")
          || trimmed.equals("extends")
          || trimmed.equals("super")) {
        continue;
      }
      int separator = Math.max(trimmed.lastIndexOf('.'), trimmed.lastIndexOf('$'));
      result.append(separator >= 0 ? trimmed.substring(separator + 1) : trimmed);
    }
    return result.toString();
  }

  private static Source findSource(Path root, String relative, int release) {
    if (Files.isDirectory(root)) {
      Path source = root.resolve(relative);
      if (!Files.isRegularFile(source)) return null;
      try {
        return new Source(source.toString(), relative, Files.readString(source));
      } catch (IOException ignored) {
        return null;
      }
    }
    if (!Files.isRegularFile(root)) return null;
    try (ZipFile zip = new ZipFile(root.toFile())) {
      ZipEntry entry = multiReleaseEntry(zip, relative, release);
      if (entry == null) {
        ZipEntry fallback = zip.stream()
            .filter(candidate -> candidate.getName().endsWith("/" + relative))
            .findFirst()
            .orElse(null);
        entry = fallback == null ? null : multiReleaseEntry(zip, fallback.getName(), release);
      }
      if (entry == null) return null;
      String content = new String(zip.getInputStream(entry).readAllBytes(), StandardCharsets.UTF_8);
      return new Source(root + "!" + entry.getName(), entry.getName(), content);
    } catch (IOException ignored) {
      return null;
    }
  }

  private static ZipEntry multiReleaseEntry(ZipFile zip, String relative, int release)
      throws IOException {
    ZipEntry selected = zip.getEntry(relative);
    if (!multiReleaseEnabled(zip)) return selected;
    for (int version = release; version >= 9; version--) {
      ZipEntry versioned = zip.getEntry("META-INF/versions/" + version + "/" + relative);
      if (versioned != null) return versioned;
    }
    return selected;
  }

  private static boolean multiReleaseEnabled(ZipFile zip) throws IOException {
    ZipEntry manifest = zip.getEntry("META-INF/MANIFEST.MF");
    if (manifest == null) return false;
    String contents =
        new String(zip.getInputStream(manifest).readAllBytes(), StandardCharsets.UTF_8);
    return contents.lines()
        .map(String::trim)
        .anyMatch(line -> line.equalsIgnoreCase("Multi-Release: true"));
  }

  private static Map<String, String> parse(
      JavaCompiler compiler, String fileName, String source) {
    Map<String, String> documentation = new HashMap<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                null,
                null,
                List.of("-proc:none"),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      DocTrees trees = DocTrees.instance(task);
      new TreePathScanner<Void, Void>() {
        @Override
        public Void scan(Tree tree, Void unused) {
          if (tree != null) {
            TreePath parent = getCurrentPath();
            TreePath path = parent == null ? new TreePath(unit) : new TreePath(parent, tree);
            DocCommentTree comment = trees.getDocCommentTree(path);
            String declarationKey = treeKey(tree);
            if (comment != null && declarationKey != null) {
              documentation.putIfAbsent(
                  declarationKey, JavadocMarkdown.render(comment.toString()));
            }
          }
          return super.scan(tree, unused);
        }
      }.scan(unit, null);
    } catch (IOException | RuntimeException ignored) {
      return Map.of();
    }
    return Map.copyOf(documentation);
  }

  private static String treeKey(Tree tree) {
    if (tree instanceof MethodTree method) {
      return "method:" + method.getName() + ":" + method.getParameters().size();
    }
    if (tree instanceof VariableTree variable) {
      return "field:" + variable.getName();
    }
    if (tree instanceof ClassTree type) {
      return "type:" + type.getSimpleName();
    }
    return null;
  }

  private static String key(Element element) {
    if (element instanceof ExecutableElement executable) {
      return "method:" + executable.getSimpleName() + ":" + executable.getParameters().size();
    }
    if (element instanceof VariableElement variable) {
      return "field:" + variable.getSimpleName();
    }
    if (element instanceof TypeElement type) {
      return "type:" + type.getSimpleName();
    }
    return "";
  }

  private static TypeElement owner(Element element) {
    TypeElement owner = element instanceof TypeElement type ? type : null;
    Element current = element.getEnclosingElement();
    while (current != null) {
      if (current instanceof TypeElement type) owner = type;
      current = current.getEnclosingElement();
    }
    return owner;
  }

  private record Source(String cacheKey, String fileName, String content) {}
}
