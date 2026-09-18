package io.github.zonnedev.jman.javac;

import com.sun.source.tree.ClassTree;
import com.sun.source.tree.AssignmentTree;
import com.sun.source.tree.CompoundAssignmentTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.ImportTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.ModuleTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.UnaryTree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.TreeScanner;
import com.sun.source.util.Trees;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import javax.lang.model.element.Element;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.PackageElement;
import javax.lang.model.element.QualifiedNameable;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.TypeMirror;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

final class JavacFrontend {
  private JavacFrontend() {}

  static ParseResult parse(String fileName, String source) {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("The jdk.compiler module is unavailable");
    }

    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavaFileObject file = new StringJavaFileObject(fileName, source);
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                null,
                diagnostics,
                List.of("-proc:none", "--release", "25"),
                null,
                List.of(file));

    List<TypeDeclaration> types = new ArrayList<>();
    String packageName = "";
    try {
      for (CompilationUnitTree unit : task.parse()) {
        packageName = unit.getPackageName() == null ? "" : unit.getPackageName().toString();
        SourcePositions positions = Trees.instance(task).getSourcePositions();
        new TreeScanner<Void, Void>() {
          public Void visitClass(ClassTree node, Void unused) {
            types.add(
                new TypeDeclaration(
                    kindName(node.getKind()),
                    node.getSimpleName().toString(),
                    positions.getStartPosition(unit, node),
                    positions.getEndPosition(unit, node)));
            return super.visitClass(node, unused);
          }
        }.scan(unit, null);
      }
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to parse in-memory Java source", exception);
    }

    List<Diagnostic> convertedDiagnostics =
        diagnostics.getDiagnostics().stream()
            .map(
                diagnostic ->
                    new Diagnostic(
                        diagnostic.getKind().name().toLowerCase(Locale.ROOT),
                        diagnostic.getCode(),
                        diagnostic.getStartPosition(),
                        diagnostic.getEndPosition(),
                        diagnostic.getLineNumber(),
                        diagnostic.getColumnNumber(),
                        diagnostic.getMessage(Locale.ROOT)))
            .toList();
    return new ParseResult(packageName, types, convertedDiagnostics);
  }

  static WorkspaceParseResult parseWorkspace(
      List<SourceInput> sources, int release, boolean enablePreview) {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("The jdk.compiler module is unavailable");
    }
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    List<String> options = new ArrayList<>(List.of("-proc:none", "--release", Integer.toString(release)));
    if (enablePreview) {
      options.add("--enable-preview");
    }
    List<JavaFileObject> files =
        sources.stream()
            .map(source -> (JavaFileObject) new StringJavaFileObject(source.fileName(), source.source()))
            .toList();
    JavacTask task =
        (JavacTask) compiler.getTask(null, null, diagnostics, options, null, files);
    Map<String, StructuralFile> results = new HashMap<>();
    try {
      for (CompilationUnitTree unit : task.parse()) {
        String fileName = sourceName(unit);
        String packageName =
            unit.getPackageName() == null ? "" : unit.getPackageName().toString();
        List<String> imports =
            unit.getImports().stream().map(ImportTree::toString).map(String::trim).toList();
        SourcePositions positions = Trees.instance(task).getSourcePositions();
        List<SemanticSymbol> symbols = new ArrayList<>();
        Deque<String> owners = new ArrayDeque<>();
        new TreeScanner<Void, Void>() {
          private String owner() {
            List<String> outerToInner = new ArrayList<>(owners.size());
            owners.descendingIterator().forEachRemaining(outerToInner::add);
            String nested = String.join(".", outerToInner);
            return packageName.isEmpty() ? nested : packageName + (nested.isEmpty() ? "" : "." + nested);
          }

          @Override
          public Void visitModule(ModuleTree node, Void unused) {
            String name = node.getName().toString();
            addStructuralSymbol(
                symbols,
                "declaration",
                "module",
                name,
                name,
                unit,
                node.getName(),
                positions,
                sources);
            return super.visitModule(node, unused);
          }

          @Override
          public Void visitClass(ClassTree node, Void unused) {
            String name = node.getSimpleName().toString();
            String qualifiedName = owner().isEmpty() ? name : owner() + "." + name;
            addStructuralSymbol(
                symbols, "declaration", kindName(node.getKind()), name, qualifiedName, unit, node,
                positions, sources);
            owners.push(name);
            try {
              return super.visitClass(node, unused);
            } finally {
              owners.pop();
            }
          }

          @Override
          public Void visitMethod(MethodTree node, Void unused) {
            String name = node.getName().toString();
            if (name.equals("<init>") && !owners.isEmpty()) {
              name = owners.peek();
            }
            addStructuralSymbol(
                symbols, "declaration", "method", name, owner() + "#" + name, unit, node,
                positions, sources);
            return super.visitMethod(node, unused);
          }

          @Override
          public Void visitVariable(VariableTree node, Void unused) {
            String name = node.getName().toString();
            addStructuralSymbol(
                symbols, "declaration", "variable", name, owner() + "#" + name, unit, node,
                positions, sources);
            return super.visitVariable(node, unused);
          }

          @Override
          public Void visitIdentifier(IdentifierTree node, Void unused) {
            addStructuralSymbol(
                symbols, "reference", "identifier", node.getName().toString(),
                node.getName().toString(), unit, node, positions, sources);
            return super.visitIdentifier(node, unused);
          }

          @Override
          public Void visitMemberSelect(MemberSelectTree node, Void unused) {
            addStructuralSymbol(
                symbols, "reference", "member_select", node.getIdentifier().toString(),
                node.toString(), unit, node, positions, sources);
            return super.visitMemberSelect(node, unused);
          }
        }.scan(unit, null);
        results.put(
            fileName,
            new StructuralFile(fileName, packageName, imports, symbols, List.of()));
      }
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to parse workspace sources", exception);
    }
    Map<String, List<Diagnostic>> diagnosticsByFile = new HashMap<>();
    for (var diagnostic : diagnostics.getDiagnostics()) {
      String fileName =
          diagnostic.getSource() == null
              ? ""
              : diagnostic.getSource() instanceof StringJavaFileObject source
                  ? source.logicalName()
                  : diagnostic.getSource().getName();
      fileName = sourceKey(fileName);
      diagnosticsByFile
          .computeIfAbsent(fileName, ignored -> new ArrayList<>())
          .add(
              new Diagnostic(
                  diagnostic.getKind().name().toLowerCase(Locale.ROOT),
                  diagnostic.getCode(),
                  diagnostic.getStartPosition(),
                  diagnostic.getEndPosition(),
                  diagnostic.getLineNumber(),
                  diagnostic.getColumnNumber(),
                  diagnostic.getMessage(Locale.ROOT)));
    }
    List<StructuralFile> ordered = new ArrayList<>();
    for (SourceInput source : sources) {
      StructuralFile result = results.get(sourceKey(source.fileName()));
      if (result == null) {
        result =
            new StructuralFile(
                source.fileName(), "", List.of(), List.of(),
                diagnosticsByFile.getOrDefault(sourceKey(source.fileName()), List.of()));
      } else {
        result =
            new StructuralFile(
                source.fileName(), result.packageName(), result.imports(), result.symbols(),
                diagnosticsByFile.getOrDefault(sourceKey(source.fileName()), List.of()));
      }
      ordered.add(result);
    }
    return new WorkspaceParseResult(ordered);
  }

  private static void addStructuralSymbol(
      List<SemanticSymbol> symbols,
      String role,
      String kind,
      String name,
      String qualifiedName,
      CompilationUnitTree unit,
      Tree tree,
      SourcePositions positions,
      List<SourceInput> sources) {
    long start = positions.getStartPosition(unit, tree);
    long end = positions.getEndPosition(unit, tree);
    if (start < 0 || end < start) {
      return;
    }
    String fileName = sourceName(unit);
    String source =
        sources.stream()
            .filter(candidate -> sourceKey(candidate.fileName()).equals(fileName))
            .map(SourceInput::source)
            .findFirst()
            .orElse("");
    long[] range = identifierRange(source, start, end, name, role.equals("declaration"));
    symbols.add(
        new SemanticSymbol(role, kind, name, qualifiedName, qualifiedName, range[0], range[1]));
  }

  private static String sourceName(CompilationUnitTree unit) {
    String name =
        unit.getSourceFile() instanceof StringJavaFileObject source
            ? source.logicalName()
            : unit.getSourceFile().getName();
    return sourceKey(name);
  }

  private static String sourceKey(String name) {
    return name.replaceFirst("^/+", "");
  }

  static SemanticResult analyze(
      String fileName,
      String source,
      List<Path> classpath,
      List<Path> sourcePath,
      int release) {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("The jdk.compiler module is unavailable");
    }

    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    try (StandardJavaFileManager files =
        compiler.getStandardFileManager(diagnostics, Locale.ROOT, null)) {
      if (!classpath.isEmpty()) {
        files.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath);
      }
      if (!sourcePath.isEmpty()) {
        files.setLocationFromPaths(StandardLocation.SOURCE_PATH, sourcePath);
      }
      return analyze(compiler, files, diagnostics, fileName, source, release);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to analyze in-memory Java source", exception);
    }
  }

  static SemanticResult analyze(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      DiagnosticCollector<JavaFileObject> diagnostics,
      String fileName,
      String source,
      int release) {
    return analyze(
        compiler, files, diagnostics, fileName, source, release, List.of(), List.of());
  }

  static SemanticResult analyze(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      DiagnosticCollector<JavaFileObject> diagnostics,
      String fileName,
      String source,
      int release,
      List<JavaFileObject> companionSources,
      List<String> compilerOptions) {
    return analyze(
        compiler,
        files,
        diagnostics,
        new StringJavaFileObject(fileName, source),
        source,
        release,
        companionSources,
        compilerOptions);
  }

  static SemanticResult analyze(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      DiagnosticCollector<JavaFileObject> diagnostics,
      JavaFileObject sourceFile,
      String source,
      int release,
      List<JavaFileObject> companionSources,
      List<String> compilerOptions) {
    List<JavaFileObject> compilationUnits = new ArrayList<>(1 + companionSources.size());
    compilationUnits.add(sourceFile);
    compilationUnits.addAll(companionSources);
    List<String> options = analysisOptions(release, compilerOptions);
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                options,
                null,
                compilationUnits);
    List<CompilationUnitTree> units = new ArrayList<>();
    try {
      task.parse().forEach(units::add);
      task.analyze();
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to analyze in-memory Java source", exception);
    }

    Trees trees = Trees.instance(task);
    List<SemanticSymbol> symbols = new ArrayList<>();
    String packageName =
        units.isEmpty() || units.get(0).getPackageName() == null
            ? ""
            : units.get(0).getPackageName().toString();
    for (CompilationUnitTree unit : units.stream().limit(1).toList()) {
      SourcePositions positions = trees.getSourcePositions();
      if (unit.getPackageName() != null) {
        TreePath packagePath = TreePath.getPath(unit, unit.getPackageName());
        Element packageElement = packagePath == null ? null : trees.getElement(packagePath);
        if (packageElement instanceof PackageElement packageDeclaration) {
          String name = packageDeclaration.getSimpleName().toString();
          long start = positions.getStartPosition(unit, unit.getPackageName());
          long end = positions.getEndPosition(unit, unit.getPackageName());
          long[] range = identifierRange(source, start, end, name, true);
          symbols.add(
              new SemanticSymbol(
                  "declaration",
                  "package",
                  name,
                  packageDeclaration.getQualifiedName().toString(),
                  SymbolIds.of(task.getElements(), task.getTypes(), packageDeclaration),
                  range[0],
                  range[1]));
        }
      }
      new TreePathScanner<Void, Void>() {
        @Override
        public Void scan(Tree tree, Void unused) {
          if (tree != null) {
            TreePath parent = getCurrentPath();
            TreePath path = parent == null ? new TreePath(unit) : new TreePath(parent, tree);
            Element element = trees.getElement(path);
            if (element != null
                && !SymbolIds.isUnresolvedRecovery(task.getElements(), element)) {
              long start = positions.getStartPosition(unit, tree);
              long end = positions.getEndPosition(unit, tree);
              if (start >= 0 && end >= start && isSymbolTree(tree)) {
                String name = element.getSimpleName().toString();
                long[] range =
                    identifierRange(source, start, end, name, isDeclaration(tree));
                String symbolId = SymbolIds.of(task.getElements(), task.getTypes(), element);
                String role =
                    isDeclaration(tree)
                        ? "declaration"
                        : isWriteAccess(path, element) ? "write" : "reference";
                symbols.add(
                    new SemanticSymbol(
                        role,
                        element.getKind().name().toLowerCase(Locale.ROOT),
                        name,
                        qualifiedName(element),
                        symbolId,
                        range[0],
                        range[1]));
                if (isDeclaration(tree) && element instanceof TypeElement typeElement) {
                  for (TypeMirror supertype : task.getTypes().directSupertypes(typeElement.asType())) {
                    Element superElement = task.getTypes().asElement(supertype);
                    if (superElement instanceof TypeElement) {
                      symbols.add(
                          new SemanticSymbol(
                              "type_edge",
                              "type",
                              superElement.getSimpleName().toString(),
                              symbolId,
                              SymbolIds.of(task.getElements(), task.getTypes(), superElement),
                              range[0],
                              range[1]));
                    }
                  }
                }
                String overrideFamily =
                    SymbolIds.overrideFamily(task.getElements(), task.getTypes(), element);
                if (!overrideFamily.isEmpty()) {
                  symbols.add(
                      new SemanticSymbol(
                          "override_family",
                          "method",
                          name,
                          overrideFamily,
                          symbolId,
                          range[0],
                          range[1]));
                }
                if (!isDeclaration(tree) && element instanceof ExecutableElement) {
                  String callerId = enclosingExecutableId(task, trees, path);
                  if (!callerId.isEmpty()) {
                    symbols.add(
                        new SemanticSymbol(
                            "call_edge",
                            "method",
                            name,
                            callerId,
                            symbolId,
                            range[0],
                            range[1]));
                  }
                }
              }
            }
          }
          return super.scan(tree, unused);
        }
      }.scan(unit, null);
    }
    return new SemanticResult(
        packageName, symbols, convertDiagnostics(diagnostics, sourceFile));
  }

  private static long[] identifierRange(
      String source, long treeStart, long treeEnd, String name, boolean declaration) {
    if (name.isEmpty() || name.startsWith("<")) {
      return new long[] {treeStart, treeEnd};
    }
    int start = Math.toIntExact(Math.min(treeStart, source.length()));
    int end = Math.toIntExact(Math.min(treeEnd, source.length()));
    int match =
        declaration
            ? source.indexOf(name, start)
            : source.lastIndexOf(name, Math.max(start, end - name.length()));
    while (match >= start && match + name.length() <= end) {
      boolean leftBoundary =
          match == 0 || !Character.isJavaIdentifierPart(source.charAt(match - 1));
      boolean rightBoundary =
          match + name.length() == source.length()
              || !Character.isJavaIdentifierPart(source.charAt(match + name.length()));
      if (leftBoundary && rightBoundary) {
        return new long[] {match, match + name.length()};
      }
      match =
          declaration
              ? source.indexOf(name, match + 1)
              : source.lastIndexOf(name, match - 1);
    }
    return new long[] {treeStart, treeEnd};
  }

  private static boolean isSymbolTree(Tree tree) {
    return isDeclaration(tree)
        || tree.getKind() == Tree.Kind.IDENTIFIER
        || tree.getKind() == Tree.Kind.MEMBER_SELECT
        || tree.getKind() == Tree.Kind.MEMBER_REFERENCE
        || tree.getKind() == Tree.Kind.NEW_CLASS;
  }

  private static boolean isDeclaration(Tree tree) {
    return tree instanceof ModuleTree
        || tree instanceof ClassTree
        || tree instanceof MethodTree
        || tree instanceof VariableTree;
  }

  private static boolean isWriteAccess(TreePath path, Element element) {
    if (!(element instanceof VariableElement) || path.getParentPath() == null) return false;
    Tree tree = path.getLeaf();
    Tree parent = path.getParentPath().getLeaf();
    if (parent instanceof AssignmentTree assignment) {
      return assignment.getVariable() == tree;
    }
    if (parent instanceof CompoundAssignmentTree assignment) {
      return assignment.getVariable() == tree;
    }
    if (parent instanceof UnaryTree unary) {
      return switch (unary.getKind()) {
        case PREFIX_INCREMENT, PREFIX_DECREMENT, POSTFIX_INCREMENT, POSTFIX_DECREMENT -> true;
        default -> false;
      };
    }
    return false;
  }

  private static String qualifiedName(Element element) {
    if (element instanceof QualifiedNameable named) {
      return named.getQualifiedName().toString();
    }
    if (element instanceof ExecutableElement executable) {
      return ownerName(executable) + "#" + executable.getSimpleName();
    }
    if (element instanceof VariableElement variable) {
      return ownerName(variable) + "#" + variable.getSimpleName();
    }
    return element.getSimpleName().toString();
  }

  private static String enclosingExecutableId(JavacTask task, Trees trees, TreePath path) {
    for (TreePath current = path.getParentPath(); current != null; current = current.getParentPath()) {
      Element owner = trees.getElement(current);
      if (current.getLeaf() instanceof MethodTree && owner instanceof ExecutableElement) {
        return SymbolIds.of(task.getElements(), task.getTypes(), owner);
      }
    }
    return "";
  }

  private static String ownerName(Element element) {
    Element owner = element.getEnclosingElement();
    while (owner != null) {
      if (owner instanceof TypeElement type) {
        return type.getQualifiedName().toString();
      }
      if (owner instanceof PackageElement packageElement) {
        return packageElement.getQualifiedName().toString();
      }
      owner = owner.getEnclosingElement();
    }
    return "";
  }

  private static List<Diagnostic> convertDiagnostics(
      DiagnosticCollector<JavaFileObject> diagnostics) {
    return convertDiagnostics(diagnostics, null);
  }

  private static List<Diagnostic> convertDiagnostics(
      DiagnosticCollector<JavaFileObject> diagnostics, JavaFileObject primary) {
    return diagnostics.getDiagnostics().stream()
        .filter(
            diagnostic ->
                primary == null
                    || diagnostic.getSource() == null
                    || diagnostic.getSource().toUri().equals(primary.toUri()))
        .map(
            diagnostic ->
                new Diagnostic(
                    diagnostic.getKind().name().toLowerCase(Locale.ROOT),
                    diagnostic.getCode(),
                    diagnostic.getStartPosition(),
                    diagnostic.getEndPosition(),
                    diagnostic.getLineNumber(),
                    diagnostic.getColumnNumber(),
                    diagnostic.getMessage(Locale.ROOT)))
        .toList();
  }

  static List<String> analysisOptions(int release, List<String> compilerOptions) {
    List<String> options = new ArrayList<>();
    if (compilerOptions.stream().noneMatch(option -> option.startsWith("-proc:"))) {
      options.add("-proc:none");
    }
    options.add("--release");
    options.add(Integer.toString(release));
    options.addAll(compilerOptions);
    return options;
  }

  private static String kindName(Tree.Kind kind) {
    return switch (kind) {
      case CLASS -> "class";
      case INTERFACE -> "interface";
      case ENUM -> "enum";
      case ANNOTATION_TYPE -> "annotation";
      case RECORD -> "record";
      default -> kind.name().toLowerCase(Locale.ROOT);
    };
  }
}
