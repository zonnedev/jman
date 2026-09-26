package io.github.zonnedev.jman.javac;

import com.sun.source.tree.ClassTree;
import com.sun.source.tree.AnnotationTree;
import com.sun.source.tree.BlockTree;
import com.sun.source.tree.CaseTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.ImportTree;
import com.sun.source.tree.ConditionalExpressionTree;
import com.sun.source.tree.BinaryTree;
import com.sun.source.tree.DoWhileLoopTree;
import com.sun.source.tree.DirectiveTree;
import com.sun.source.tree.EnhancedForLoopTree;
import com.sun.source.tree.ForLoopTree;
import com.sun.source.tree.IfTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.ModuleTree;
import com.sun.source.tree.ModifiersTree;
import com.sun.source.tree.NewArrayTree;
import com.sun.source.tree.LambdaExpressionTree;
import com.sun.source.tree.ParenthesizedTree;
import com.sun.source.tree.StatementTree;
import com.sun.source.tree.TryTree;
import com.sun.source.tree.WhileLoopTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.TreePath;
import com.sun.source.util.Trees;
import com.sun.tools.javac.api.BasicJavacTask;
import com.sun.tools.javac.parser.Lexer;
import com.sun.tools.javac.parser.ScannerFactory;
import com.sun.tools.javac.parser.Tokens;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.Modifier;
import javax.lang.model.element.NestingKind;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.ExecutableType;
import javax.lang.model.type.TypeKind;
import javax.lang.model.type.TypeMirror;
import javax.lang.model.util.ElementFilter;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

/** The canonical JMAN Java source formatter. */
final class JavaFormatter {
  private static final String INDENT = "  ";
  private static final Set<String> CONTROL_PARENTHESIS =
      Set.of("if", "for", "while", "switch", "catch", "synchronized", "try");
  private static final Set<String> BINARY_OPERATORS =
      Set.of(
          "=", "+", "-", "*", "/", "%", "==", "!=", "<", ">", "<=", ">=", "&&",
          "||", "&", "|", "^", "<<", ">>", ">>>", "+=", "-=", "*=", "/=", "%=",
          "&=", "|=", "^=", "<<=", ">>=", ">>>=", "->");

  private JavaFormatter() {}

  static FormatResult format(
      String fileName, String source, List<Path> classpath, List<Path> sourcePath, int release) {
    return format(fileName, source, classpath, sourcePath, List.of(), release);
  }

  static FormatResult format(
      String fileName,
      String source,
      List<Path> classpath,
      List<Path> sourcePath,
      List<Path> modulePath,
      int release) {
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    if (compiler == null) {
      throw new IllegalStateException("The jdk.compiler module is unavailable");
    }
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    try (StandardJavaFileManager files =
        compiler.getStandardFileManager(diagnostics, Locale.ROOT, null)) {
      if (!classpath.isEmpty()) files.setLocationFromPaths(StandardLocation.CLASS_PATH, classpath);
      if (!sourcePath.isEmpty()) files.setLocationFromPaths(StandardLocation.SOURCE_PATH, sourcePath);
      if (!modulePath.isEmpty()) files.setLocationFromPaths(StandardLocation.MODULE_PATH, modulePath);
      return format(compiler, files, fileName, source, release, List.of());
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to format Java source", exception);
    }
  }

  static FormatResult format(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    List<Diagnostic> directiveDiagnostics = directiveDiagnostics(source);
    if (!directiveDiagnostics.isEmpty()) return new FormatResult(source, directiveDiagnostics);
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavaFileObject file = new StringJavaFileObject(fileName, source);
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(file));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      List<Diagnostic> parseDiagnostics = convertErrors(diagnostics);
      if (!parseDiagnostics.isEmpty()) return new FormatResult(source, parseDiagnostics);

      Trees trees = Trees.instance(task);
      SourcePositions positions = trees.getSourcePositions();
      boolean attributionComplete = true;
      try {
        task.analyze();
      } catch (RuntimeException ignored) {
        // Attribution is best effort. Syntax-only formatting remains available.
        attributionComplete = false;
      }
      attributionComplete &= convertErrors(diagnostics).isEmpty();
      if (attributionComplete) {
        List<Diagnostic> importDirectiveDiagnostics =
            importDirectiveDiagnostics(source, unit, trees, positions);
        if (!importDirectiveDiagnostics.isEmpty()) {
          return new FormatResult(source, importDirectiveDiagnostics);
        }
      }

      List<Edit> edits = structuralEdits(source, unit, positions, task, trees);
      String normalized = applyEdits(source, edits);
      normalized =
          normalizeControlBraces(
              compiler, files, fileName, normalized, release, compilerOptions);
      normalized =
          normalizeModifiers(
              compiler, files, fileName, normalized, release, compilerOptions);
      normalized =
          normalizeImports(
              compiler, files, fileName, normalized, release, compilerOptions);
      normalized =
          normalizeLongExpressionLambdas(
              compiler, files, fileName, normalized, release, compilerOptions);
      String formatted = formatTokens(normalized, task);
      formatted = applyJjfsLayout(compiler, files, fileName, formatted, release, compilerOptions);
      List<Diagnostic> validation = validate(compiler, files, fileName, formatted, release);
      return validation.isEmpty()
          ? new FormatResult(formatted, List.of())
          : new FormatResult(source, validation);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to format Java source", exception);
    }
  }

  static List<String> formatOptions(int release, List<String> compilerOptions) {
    List<String> options =
        new ArrayList<>(List.of("-proc:none", "--release", Integer.toString(release)));
    if (compilerOptions.contains("--enable-preview")) options.add("--enable-preview");
    return options;
  }

  private static List<Diagnostic> validate(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, List.of()),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      task.parse();
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to validate formatted Java source", exception);
    }
    return convertErrors(diagnostics);
  }

  private static List<Diagnostic> convertErrors(
      DiagnosticCollector<JavaFileObject> diagnostics) {
    return diagnostics.getDiagnostics().stream()
        .filter(diagnostic -> diagnostic.getKind() == javax.tools.Diagnostic.Kind.ERROR)
        .map(
            diagnostic ->
                new Diagnostic(
                    "error",
                    diagnostic.getCode(),
                    diagnostic.getStartPosition(),
                    diagnostic.getEndPosition(),
                    diagnostic.getLineNumber(),
                    diagnostic.getColumnNumber(),
                    diagnostic.getMessage(Locale.ROOT)))
        .toList();
  }

  private static List<Diagnostic> directiveDiagnostics(String source) {
    List<Diagnostic> diagnostics = new ArrayList<>();
    boolean disabled = false;
    int disabledStart = -1;
    long line = 1;
    int offset = 0;
    while (offset <= source.length()) {
      int end = source.indexOf('\n', offset);
      if (end < 0) end = source.length();
      String text = source.substring(offset, end).strip();
      boolean off = text.equals("// jjfs: off");
      boolean on = text.equals("// jjfs: on");
      boolean preferredImport =
          text.matches("import\\s+[A-Za-z_$][\\w$]*(?:\\.[A-Za-z_$][\\w$]*)+;\\s*// jjfs: prefer-import");
      boolean jjfsDirective = text.contains("// jjfs:");
      if (off && disabled) {
        diagnostics.add(
            jjfsDiagnostic(offset, end, line, "nested // jjfs: off directives are not allowed"));
      } else if (off) {
        disabled = true;
        disabledStart = offset;
      } else if (on && !disabled) {
        diagnostics.add(
            jjfsDiagnostic(offset, end, line, "// jjfs: on has no matching // jjfs: off"));
      } else if (on) {
        disabled = false;
        disabledStart = -1;
      } else if (jjfsDirective && !preferredImport) {
        diagnostics.add(jjfsDiagnostic(offset, end, line, "invalid JJFS directive"));
      }
      if (end == source.length()) break;
      offset = end + 1;
      line++;
    }
    if (disabled) {
      long disabledLine = 1 + source.substring(0, disabledStart).chars().filter(c -> c == '\n').count();
      int end = source.indexOf('\n', disabledStart);
      if (end < 0) end = source.length();
      diagnostics.add(
          jjfsDiagnostic(
              disabledStart, end, disabledLine, "// jjfs: off has no matching // jjfs: on"));
    }
    return diagnostics;
  }

  private static Diagnostic jjfsDiagnostic(int start, int end, long line, String message) {
    return new Diagnostic("error", "jjfs.err.directive", start, end, line, 1, message);
  }

  private static List<Diagnostic> importDirectiveDiagnostics(
      String source,
      CompilationUnitTree unit,
      Trees trees,
      SourcePositions positions) {
    Map<String, Set<String>> candidates = referencedTopLevelTypes(unit, trees);
    Map<String, List<ImportTree>> preferences = new HashMap<>();
    for (ImportTree imported : unit.getImports()) {
      if (!preferredImportDirective(source, imported, unit, positions)) continue;
      String qualified = imported.getQualifiedIdentifier().toString();
      int separator = qualified.lastIndexOf('.');
      String simple = separator < 0 ? qualified : qualified.substring(separator + 1);
      preferences.computeIfAbsent(simple, ignored -> new ArrayList<>()).add(imported);
    }
    List<Diagnostic> diagnostics = new ArrayList<>();
    for (Map.Entry<String, List<ImportTree>> entry : preferences.entrySet()) {
      List<ImportTree> imports = entry.getValue();
      if (imports.size() > 1) {
        for (ImportTree imported : imports) {
          diagnostics.add(
              importDirectiveDiagnostic(
                  source,
                  unit,
                  positions,
                  imported,
                  "multiple preferred imports compete for " + entry.getKey()));
        }
        continue;
      }
      ImportTree imported = imports.get(0);
      String qualified = imported.getQualifiedIdentifier().toString();
      Set<String> conflicting = candidates.getOrDefault(entry.getKey(), Set.of());
      if (imported.isStatic()
          || qualified.endsWith(".*")
          || conflicting.size() < 2
          || !conflicting.contains(qualified)) {
        diagnostics.add(
            importDirectiveDiagnostic(
                source,
                unit,
                positions,
                imported,
                "preferred import must be a used explicit type in a simple-name conflict"));
      }
    }
    return diagnostics;
  }

  private static Map<String, Set<String>> referencedTopLevelTypes(
      CompilationUnitTree unit, Trees trees) {
    Map<String, Set<String>> candidates = new HashMap<>();
    new TreePathScanner<Void, Void>() {
      @Override
      public Void visitImport(ImportTree node, Void unused) {
        return null;
      }

      @Override
      public Void visitIdentifier(IdentifierTree node, Void unused) {
        addCandidate(trees.getElement(getCurrentPath()), candidates);
        return super.visitIdentifier(node, unused);
      }

      @Override
      public Void visitMemberSelect(MemberSelectTree node, Void unused) {
        addCandidate(trees.getElement(getCurrentPath()), candidates);
        return super.visitMemberSelect(node, unused);
      }
    }.scan(unit, null);
    return candidates;
  }

  private static Map<String, Set<String>> simplyReferencedTopLevelTypes(
      CompilationUnitTree unit, Trees trees) {
    Map<String, Set<String>> references = new HashMap<>();
    new TreePathScanner<Void, Void>() {
      @Override
      public Void visitImport(ImportTree node, Void unused) {
        return null;
      }

      @Override
      public Void visitIdentifier(IdentifierTree node, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        if (element instanceof TypeElement type) {
          TypeElement top = topLevelType(type);
          if (top != null && !top.getSimpleName().isEmpty()) {
            references
                .computeIfAbsent(top.getSimpleName().toString(), ignored -> new LinkedHashSet<>())
                .add(top.getQualifiedName().toString());
          }
        }
        return super.visitIdentifier(node, unused);
      }
    }.scan(unit, null);
    return references;
  }

  private static Diagnostic importDirectiveDiagnostic(
      String source,
      CompilationUnitTree unit,
      SourcePositions positions,
      ImportTree imported,
      String message) {
    int start = position(positions.getStartPosition(unit, imported));
    int end = position(positions.getEndPosition(unit, imported));
    long line = 1 + source.substring(0, Math.max(0, start)).chars().filter(c -> c == '\n').count();
    return jjfsDiagnostic(start, end, line, message);
  }

  private static String normalizeImports(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      if (!convertErrors(diagnostics).isEmpty()) return source;
      Trees trees = Trees.instance(task);
      boolean attributionComplete = true;
      try {
        task.analyze();
      } catch (RuntimeException ignored) {
        attributionComplete = false;
      }
      attributionComplete &= convertErrors(diagnostics).isEmpty();
      SourcePositions positions = trees.getSourcePositions();
      ImportChoices importChoices = importChoices(source, unit, trees, positions);
      StaticRewrite staticRewrite =
          attributionComplete
              ? staticImportRewrites(source, unit, trees, positions, importChoices)
              : new StaticRewrite(List.of(), Set.of());
      List<Edit> edits = new ArrayList<>(staticRewrite.edits());
      Edit imports =
          expandedImports(
              source,
              unit,
              trees,
              task,
              positions,
              attributionComplete,
              staticRewrite.ownerImports(),
              importChoices);
      if (imports != null) edits.add(imports);
      return applyEdits(source, edits);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to normalize Java imports", exception);
    }
  }

  private static String normalizeControlBraces(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      if (!convertErrors(diagnostics).isEmpty()) return source;
      SourcePositions positions = Trees.instance(task).getSourcePositions();
      List<Edit> edits = new ArrayList<>();
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitIf(IfTree statement, Void unused) {
          addBraces(statement.getThenStatement());
          if (statement.getElseStatement() != null
              && !(statement.getElseStatement() instanceof IfTree)) {
            addBraces(statement.getElseStatement());
          }
          return super.visitIf(statement, unused);
        }

        @Override
        public Void visitWhileLoop(WhileLoopTree statement, Void unused) {
          addBraces(statement.getStatement());
          return super.visitWhileLoop(statement, unused);
        }

        @Override
        public Void visitDoWhileLoop(DoWhileLoopTree statement, Void unused) {
          addBraces(statement.getStatement());
          return super.visitDoWhileLoop(statement, unused);
        }

        @Override
        public Void visitForLoop(ForLoopTree statement, Void unused) {
          addBraces(statement.getStatement());
          return super.visitForLoop(statement, unused);
        }

        @Override
        public Void visitEnhancedForLoop(EnhancedForLoopTree statement, Void unused) {
          addBraces(statement.getStatement());
          return super.visitEnhancedForLoop(statement, unused);
        }

        private void addBraces(StatementTree statement) {
          if (statement == null || statement instanceof BlockTree) return;
          int start = position(positions.getStartPosition(unit, statement));
          int end = position(positions.getEndPosition(unit, statement));
          if (start < 0 || end <= start || disabledAt(source, start)) return;
          edits.add(new Edit(end, end, "}"));
          edits.add(new Edit(start, start, "{"));
        }
      }.scan(unit, null);
      return applyEdits(source, edits);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to normalize control-flow braces", exception);
    }
  }

  private static String normalizeLongExpressionLambdas(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      if (!convertErrors(diagnostics).isEmpty()) return source;
      Trees trees = Trees.instance(task);
      try {
        task.analyze();
      } catch (RuntimeException ignored) {
        return source;
      }
      if (!convertErrors(diagnostics).isEmpty()) return source;
      SourcePositions positions = trees.getSourcePositions();
      List<Edit> edits = new ArrayList<>();
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitLambdaExpression(LambdaExpressionTree lambda, Void unused) {
          if (lambda.getBodyKind() != LambdaExpressionTree.BodyKind.EXPRESSION) {
            return super.visitLambdaExpression(lambda, unused);
          }
          int start = position(positions.getStartPosition(unit, lambda));
          int bodyStart = position(positions.getStartPosition(unit, lambda.getBody()));
          int bodyEnd = position(positions.getEndPosition(unit, lambda.getBody()));
          if (start < 0 || bodyStart < 0 || bodyEnd <= bodyStart || disabledAt(source, start)) {
            return super.visitLambdaExpression(lambda, unused);
          }
          if (compactWidth(source, start, bodyEnd) <= 60) {
            return super.visitLambdaExpression(lambda, unused);
          }
          Boolean returnsVoid = lambdaReturnsVoid(getCurrentPath(), trees, task);
          if (returnsVoid == null) return super.visitLambdaExpression(lambda, unused);
          String expression = source.substring(bodyStart, bodyEnd);
          String statement = returnsVoid ? expression + ";" : "return " + expression + ";";
          edits.add(new Edit(bodyStart, bodyEnd, "{ " + statement + " }"));
          return null;
        }
      }.scan(unit, null);
      return applyEdits(source, edits);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to normalize Java lambdas", exception);
    }
  }

  private static Boolean lambdaReturnsVoid(TreePath path, Trees trees, JavacTask task) {
    TypeMirror target = trees.getTypeMirror(path);
    if (!(target instanceof DeclaredType declared)
        || !(declared.asElement() instanceof TypeElement type)) {
      return null;
    }
    List<ExecutableElement> descriptors =
        ElementFilter.methodsIn(task.getElements().getAllMembers(type)).stream()
            .filter(method -> method.getModifiers().contains(Modifier.ABSTRACT))
            .filter(method -> !method.getModifiers().contains(Modifier.STATIC))
            .filter(
                method ->
                    !(method.getEnclosingElement() instanceof TypeElement owner)
                        || !owner.getQualifiedName().contentEquals("java.lang.Object"))
            .toList();
    if (descriptors.isEmpty()) return null;
    ExecutableElement descriptor = descriptors.get(0);
    for (ExecutableElement candidate : descriptors) {
      if (!candidate.getSimpleName().contentEquals(descriptor.getSimpleName())
          || candidate.getParameters().size() != descriptor.getParameters().size()) {
        return null;
      }
    }
    TypeMirror member = task.getTypes().asMemberOf(declared, descriptor);
    if (!(member instanceof ExecutableType executable)) return null;
    return executable.getReturnType().getKind() == TypeKind.VOID;
  }

  private static boolean disabledAt(String source, int position) {
    int off = source.lastIndexOf("// jjfs: off", position);
    if (off < 0) return false;
    int on = source.lastIndexOf("// jjfs: on", position);
    return on < off;
  }

  private static String normalizeModifiers(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      if (!convertErrors(diagnostics).isEmpty()) return source;
      SourcePositions positions = Trees.instance(task).getSourcePositions();
      List<Edit> edits = new ArrayList<>();
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitClass(ClassTree declaration, Void unused) {
          addModifierEdits(declaration, declaration.getModifiers());
          return super.visitClass(declaration, unused);
        }

        @Override
        public Void visitMethod(MethodTree declaration, Void unused) {
          addModifierEdits(declaration, declaration.getModifiers());
          return super.visitMethod(declaration, unused);
        }

        @Override
        public Void visitVariable(VariableTree declaration, Void unused) {
          TreePath parent = getCurrentPath().getParentPath();
          if (parent != null && parent.getLeaf() instanceof MethodTree method
              && method.getParameters().contains(declaration)) {
            return super.visitVariable(declaration, unused);
          }
          if (parent != null && parent.getLeaf() instanceof ClassTree type
              && type.getKind() == Tree.Kind.RECORD) {
            return super.visitVariable(declaration, unused);
          }
          addModifierEdits(declaration, declaration.getModifiers());
          return super.visitVariable(declaration, unused);
        }

        private void addModifierEdits(Tree declaration, ModifiersTree modifiers) {
          int declarationStart = position(positions.getStartPosition(unit, declaration));
          int modifiersStart = position(positions.getStartPosition(unit, modifiers));
          int modifiersEnd = position(positions.getEndPosition(unit, modifiers));
          if (declarationStart < 0 || modifiersStart < 0 || modifiersEnd <= modifiersStart) return;
          if (disabledAt(source, declarationStart)) return;
          int indent = braceDepth(source, task, declarationStart) * 2;
          List<? extends com.sun.source.tree.AnnotationTree> annotations = modifiers.getAnnotations();
          for (int index = 1; index < annotations.size(); index++) {
            int previousEnd = position(positions.getEndPosition(unit, annotations.get(index - 1)));
            int currentStart = position(positions.getStartPosition(unit, annotations.get(index)));
            if (previousEnd >= 0 && currentStart >= previousEnd) {
              edits.add(new Edit(previousEnd, currentStart, "\n" + " ".repeat(indent)));
            }
          }

          List<TokenSpan> modifierTokens =
              modifierTokens(source, task, modifiersStart, modifiersEnd, annotations, unit, positions);
          if (!annotations.isEmpty()) {
            int annotationEnd =
                position(positions.getEndPosition(unit, annotations.get(annotations.size() - 1)));
            int following =
                modifierTokens.isEmpty()
                    ? modifiersEnd
                    : modifierTokens.get(0).start();
            if (annotationEnd >= 0 && following >= annotationEnd) {
              edits.add(new Edit(annotationEnd, following, "\n" + " ".repeat(indent)));
            }
          }
          if (modifierTokens.size() < 2) return;
          int first = modifierTokens.get(0).start();
          int last = modifierTokens.get(modifierTokens.size() - 1).end();
          boolean annotationInside =
              annotations.stream()
                  .mapToInt(annotation -> position(positions.getStartPosition(unit, annotation)))
                  .anyMatch(annotationStart -> annotationStart > first && annotationStart < last);
          if (annotationInside) return;
          String canonical =
              MODIFIER_ORDER.stream()
                  .filter(modifiers.getFlags()::contains)
                  .map(JavaFormatter::modifierText)
                  .collect(java.util.stream.Collectors.joining(" "));
          edits.add(new Edit(first, last, canonical));
        }
      }.scan(unit, null);
      return applyEdits(source, edits);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to normalize Java modifiers", exception);
    }
  }

  private static final List<Modifier> MODIFIER_ORDER =
      List.of(
          Modifier.PUBLIC,
          Modifier.PROTECTED,
          Modifier.PRIVATE,
          Modifier.ABSTRACT,
          Modifier.DEFAULT,
          Modifier.STATIC,
          Modifier.SEALED,
          Modifier.NON_SEALED,
          Modifier.FINAL,
          Modifier.TRANSIENT,
          Modifier.VOLATILE,
          Modifier.SYNCHRONIZED,
          Modifier.NATIVE,
          Modifier.STRICTFP);

  private static List<TokenSpan> modifierTokens(
      String source,
      JavacTask task,
      int start,
      int end,
      List<? extends com.sun.source.tree.AnnotationTree> annotations,
      CompilationUnitTree unit,
      SourcePositions positions) {
    Set<String> names =
        MODIFIER_ORDER.stream().map(JavaFormatter::modifierText).collect(java.util.stream.Collectors.toSet());
    List<TokenSpan> spans = new ArrayList<>();
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) break;
      if (token.pos < start) continue;
      boolean inAnnotation =
          annotations.stream()
              .anyMatch(
                  annotation -> {
                    int annotationStart = position(positions.getStartPosition(unit, annotation));
                    int annotationEnd = position(positions.getEndPosition(unit, annotation));
                    return token.pos >= annotationStart && token.endPos <= annotationEnd;
                  });
      String text = source.substring(token.pos, token.endPos);
      if (!inAnnotation && names.contains(text)) spans.add(new TokenSpan(token.pos, token.endPos));
    }
    return spans;
  }

  private static String modifierText(Modifier modifier) {
    return modifier == Modifier.NON_SEALED ? "non-sealed" : modifier.toString();
  }

  private static List<Edit> structuralEdits(
      String source,
      CompilationUnitTree unit,
      SourcePositions positions,
      JavacTask task,
      Trees trees) {
    List<Edit> edits = new ArrayList<>();
    for (Tree declaration : unit.getTypeDecls()) {
      if (declaration instanceof ClassTree type) {
        Edit members = orderedMembers(source, unit, type, positions, task, trees);
        if (members != null) edits.add(members);
      }
    }
    if (unit.getModule() != null) {
      Edit module = orderedModuleDirectives(source, unit, unit.getModule(), positions, task);
      if (module != null) edits.add(module);
    }
    if (unit.getPackage() != null && unit.getImports().isEmpty() && !unit.getTypeDecls().isEmpty()) {
      int packageEnd = Math.toIntExact(positions.getEndPosition(unit, unit.getPackage()));
      int typeStart = Math.toIntExact(positions.getStartPosition(unit, unit.getTypeDecls().get(0)));
      if (packageEnd >= 0
          && typeStart > packageEnd
          && source.substring(packageEnd, typeStart).isBlank()) {
        edits.add(new Edit(packageEnd, typeStart, "\n\n"));
      }
    }
    return edits;
  }

  private static Edit orderedModuleDirectives(
      String source,
      CompilationUnitTree unit,
      ModuleTree module,
      SourcePositions positions,
      JavacTask task) {
    if (module.getDirectives().size() < 2) return null;
    int moduleStart = position(positions.getStartPosition(unit, module));
    int moduleEnd = position(positions.getEndPosition(unit, module));
    int firstStart = position(positions.getStartPosition(unit, module.getDirectives().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, module.getDirectives().get(module.getDirectives().size() - 1)));
    if (moduleStart < 0 || moduleEnd <= moduleStart || firstStart < 0 || lastEnd < 0) return null;
    int open = lastTokenPosition(source, task, moduleStart, firstStart, "{");
    int close = firstTokenPosition(source, task, lastEnd, moduleEnd, "}");
    if (open < 0 || close < 0) return null;

    List<ModuleDirectiveSource> directives = new ArrayList<>();
    int cursor = open + 1;
    for (int index = 0; index < module.getDirectives().size(); index++) {
      DirectiveTree directive = module.getDirectives().get(index);
      int start = position(positions.getStartPosition(unit, directive));
      int end = position(positions.getEndPosition(unit, directive));
      if (start < 0 || end <= start) return null;
      directives.add(
          new ModuleDirectiveSource(
              moduleDirectiveCategory(directive),
              directive.toString(),
              index,
              source.substring(cursor, end).strip()));
      cursor = end;
    }
    directives.sort(
        Comparator.comparingInt(ModuleDirectiveSource::category)
            .thenComparing(ModuleDirectiveSource::key)
            .thenComparingInt(ModuleDirectiveSource::originalIndex));
    StringBuilder body = new StringBuilder("\n");
    ModuleDirectiveSource previous = null;
    for (ModuleDirectiveSource directive : directives) {
      if (previous != null) {
        body.append(previous.category() == directive.category() ? '\n' : "\n\n");
      }
      body.append(directive.source());
      previous = directive;
    }
    body.append('\n');
    return new Edit(open + 1, close, body.toString());
  }

  private static int moduleDirectiveCategory(DirectiveTree directive) {
    return switch (directive.getKind()) {
      case REQUIRES -> 0;
      case EXPORTS -> 1;
      case OPENS -> 2;
      case USES -> 3;
      case PROVIDES -> 4;
      default -> 5;
    };
  }

  private static String applyEdits(String source, List<Edit> edits) {
    edits.sort(
        Comparator.comparingInt(Edit::start)
            .reversed()
            .thenComparing(Comparator.comparingInt(Edit::end).reversed()));
    StringBuilder normalized = new StringBuilder(source);
    for (Edit edit : edits) normalized.replace(edit.start(), edit.end(), edit.replacement());
    return normalized.toString();
  }

  private static ImportChoices importChoices(
      String source,
      CompilationUnitTree unit,
      Trees trees,
      SourcePositions positions) {
    Map<String, Set<String>> candidates = referencedTopLevelTypes(unit, trees);
    Map<String, Set<String>> simpleReferences = simplyReferencedTopLevelTypes(unit, trees);

    Map<String, String> preferred = new HashMap<>();
    for (ImportTree imported : unit.getImports()) {
      if (imported.isStatic()) continue;
      String qualified = imported.getQualifiedIdentifier().toString();
      if (qualified.endsWith(".*") || !preferredImportDirective(source, imported, unit, positions)) {
        continue;
      }
      int separator = qualified.lastIndexOf('.');
      if (separator >= 0) preferred.put(qualified.substring(separator + 1), qualified);
    }

    Map<String, String> selected = new HashMap<>();
    for (Map.Entry<String, Set<String>> entry : candidates.entrySet()) {
      String choice = entry.getValue().stream().sorted().findFirst().orElseThrow();
      Set<String> boundSimpleReferences =
          simpleReferences.getOrDefault(entry.getKey(), Set.of());
      if (boundSimpleReferences.size() == 1) {
        String boundType = boundSimpleReferences.iterator().next();
        if (entry.getValue().contains(boundType)) choice = boundType;
      }
      String preference = preferred.get(entry.getKey());
      if (preference != null && entry.getValue().contains(preference)) choice = preference;
      selected.put(entry.getKey(), choice);
    }
    return new ImportChoices(selected, Set.copyOf(preferred.values()));
  }

  private static void addCandidate(Element element, Map<String, Set<String>> candidates) {
    TypeElement type = element instanceof TypeElement found ? found : enclosingType(element);
    TypeElement top = type == null ? null : topLevelType(type);
    if (top == null || top.getSimpleName().isEmpty()) return;
    candidates
        .computeIfAbsent(top.getSimpleName().toString(), ignored -> new LinkedHashSet<>())
        .add(top.getQualifiedName().toString());
  }

  private static boolean preferredImportDirective(
      String source,
      ImportTree imported,
      CompilationUnitTree unit,
      SourcePositions positions) {
    int end = position(positions.getEndPosition(unit, imported));
    if (end < 0 || end > source.length()) return false;
    int lineEnd = source.indexOf('\n', end);
    if (lineEnd < 0) lineEnd = source.length();
    return source.substring(end, lineEnd).strip().equals("// jjfs: prefer-import");
  }

  private static Edit expandedImports(
      String source,
      CompilationUnitTree unit,
      Trees trees,
      JavacTask task,
      SourcePositions positions,
      boolean attributionComplete,
      Set<String> additionalOrdinaryImports,
      ImportChoices importChoices) {
    if (unit.getImports().isEmpty() && additionalOrdinaryImports.isEmpty()) return null;
    Set<TypeElement> referenced = new LinkedHashSet<>();
    Set<Element> referencedElements = new LinkedHashSet<>();
    Set<String> identifiers = new LinkedHashSet<>();
    new TreePathScanner<Void, Void>() {
      @Override
      public Void visitImport(ImportTree node, Void unused) {
        return null;
      }

      @Override
      public Void visitIdentifier(IdentifierTree node, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        addType(element, referenced);
        if (element != null) referencedElements.add(element);
        identifiers.add(node.getName().toString());
        return super.visitIdentifier(node, unused);
      }

      @Override
      public Void visitMemberSelect(MemberSelectTree node, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        addType(element, referenced);
        if (element != null) referencedElements.add(element);
        return super.visitMemberSelect(node, unused);
      }
    }.scan(unit, null);

    Map<String, Set<String>> wildcardTypes = new HashMap<>();
    Map<String, Set<String>> wildcardStatics = new HashMap<>();
    for (ImportTree imported : unit.getImports()) {
      String name = imported.getQualifiedIdentifier().toString();
      if (!imported.isStatic() && name.endsWith(".*")) {
        wildcardTypes.put(name.substring(0, name.length() - 2), new LinkedHashSet<>());
      } else if (imported.isStatic() && name.endsWith(".*")) {
        wildcardStatics.put(name.substring(0, name.length() - 2), new LinkedHashSet<>());
      }
    }
    for (TypeElement type : referenced) {
      String packageName = task.getElements().getPackageOf(type).getQualifiedName().toString();
      Set<String> expansion = wildcardTypes.get(packageName);
      if (expansion != null) expansion.add(type.getQualifiedName().toString());
    }
    // Attribution can be incomplete when another project source is currently
    // invalid. Resolve syntactically referenced simple names independently so
    // a wildcard import is never destructively removed for that reason.
    for (Map.Entry<String, Set<String>> wildcard : wildcardTypes.entrySet()) {
      for (String identifier : identifiers) {
        TypeElement type = task.getElements().getTypeElement(wildcard.getKey() + "." + identifier);
        if (type != null && type.getNestingKind() == NestingKind.TOP_LEVEL) {
          wildcard.getValue().add(type.getQualifiedName().toString());
        }
      }
    }
    for (Element element : referencedElements) {
      if (element.getEnclosingElement() instanceof TypeElement owner) {
        Set<String> expansion = wildcardStatics.get(owner.getQualifiedName().toString());
        if (expansion != null) {
          expansion.add(owner.getQualifiedName() + "." + element.getSimpleName());
        }
      }
    }

    Set<String> ordinary = new LinkedHashSet<>();
    Set<String> statics = new LinkedHashSet<>();
    for (ImportTree imported : unit.getImports()) {
      String name = imported.getQualifiedIdentifier().toString();
      if (!imported.isStatic() && name.endsWith(".*")) {
        if (!attributionComplete) {
          ordinary.add(name);
          continue;
        }
        Set<String> expansion =
            wildcardTypes.getOrDefault(name.substring(0, name.length() - 2), Set.of());
        ordinary.addAll(expansion);
      } else if (imported.isStatic() && name.endsWith(".*")) {
        if (!attributionComplete) {
          statics.add(name);
          continue;
        }
        // A complete attribution pass rewrites every referenced member to an
        // owner-qualified access, so the static wildcard disappears.
      } else if (imported.isStatic()) {
        if (!attributionComplete) statics.add(name);
      } else {
        boolean used =
            referenced.stream()
                .map(type -> type.getQualifiedName().toString())
                .anyMatch(qualified -> qualified.equals(name) || qualified.startsWith(name + "."));
        if (!attributionComplete || used) ordinary.add(name);
      }
    }
    ordinary.addAll(additionalOrdinaryImports);
    ordinary.removeIf(name -> !importChoices.selected(name));
    String currentPackage = unit.getPackageName() == null ? "" : unit.getPackageName().toString();
    ordinary.removeIf(
        name -> {
          int separator = name.lastIndexOf('.');
          String packageName = separator < 0 ? "" : name.substring(0, separator);
          return packageName.equals("java.lang") || packageName.equals(currentPackage);
        });
    List<String> lines = new ArrayList<>();
    ordinary.stream()
        .sorted()
        .map(
            name ->
                "import "
                    + name
                    + ";"
                    + (importChoices.preferred(name) ? " // jjfs: prefer-import" : ""))
        .forEach(lines::add);
    if (!ordinary.isEmpty() && !statics.isEmpty()) lines.add("");
    statics.stream().sorted().map(name -> "import static " + name + ";").forEach(lines::add);

    int start;
    int end;
    if (unit.getImports().isEmpty()) {
      start = unit.getPackage() == null ? 0 : Math.toIntExact(positions.getEndPosition(unit, unit.getPackage()));
      end = start;
    } else {
      start = Math.toIntExact(positions.getStartPosition(unit, unit.getImports().get(0)));
      end =
          Math.toIntExact(
              positions.getEndPosition(unit, unit.getImports().get(unit.getImports().size() - 1)));
      ImportTree lastImport = unit.getImports().get(unit.getImports().size() - 1);
      if (preferredImportDirective(source, lastImport, unit, positions)) {
        int lineEnd = source.indexOf('\n', end);
        end = lineEnd < 0 ? source.length() : lineEnd;
      }
    }
    String leading = unit.getPackage() == null ? "" : "\n\n";
    return new Edit(
        start, end, lines.isEmpty() ? "" : leading + String.join("\n", lines) + "\n\n");
  }

  private static void addType(Element element, Set<TypeElement> referenced) {
    if (element instanceof TypeElement type) referenced.add(type);
  }

  private static StaticRewrite staticImportRewrites(
      String source,
      CompilationUnitTree unit,
      Trees trees,
      SourcePositions positions,
      ImportChoices importChoices) {
    Map<String, String> explicit = new HashMap<>();
    Set<String> wildcards = new HashSet<>();
    for (ImportTree imported : unit.getImports()) {
      if (!imported.isStatic()) continue;
      String name = imported.getQualifiedIdentifier().toString();
      if (name.endsWith(".*")) {
        wildcards.add(name.substring(0, name.length() - 2));
      } else {
        int separator = name.lastIndexOf('.');
        if (separator > 0) explicit.put(name.substring(separator + 1), name.substring(0, separator));
      }
    }
    List<Edit> edits = new ArrayList<>();
    Set<String> owners = new LinkedHashSet<>();
    new TreePathScanner<Void, Void>() {
      @Override
      public Void visitImport(ImportTree node, Void unused) {
        return null;
      }

      @Override
      public Void visitIdentifier(IdentifierTree identifier, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        if (element == null || !staticMember(element)) {
          return super.visitIdentifier(identifier, unused);
        }
        if (element.getKind() == ElementKind.ENUM_CONSTANT && insideCaseLabel(getCurrentPath())) {
          return super.visitIdentifier(identifier, unused);
        }
        String simpleName = identifier.getName().toString();
        TypeElement elementOwner = enclosingType(element);
        if (elementOwner == null) return super.visitIdentifier(identifier, unused);
        String importedOwner = explicit.get(simpleName);
        if (importedOwner == null) {
          String qualifiedOwner = elementOwner.getQualifiedName().toString();
          if (wildcards.contains(qualifiedOwner)) importedOwner = qualifiedOwner;
        }
        if (importedOwner == null) return super.visitIdentifier(identifier, unused);

        TypeElement importedType = trees.getElement(getCurrentPath()) instanceof TypeElement type
            ? enclosingType(type)
            : elementOwner;
        TypeElement top = topLevelType(importedType);
        if (top == null) return super.visitIdentifier(identifier, unused);
        boolean selected = importChoices.selected(top.getQualifiedName().toString());
        String qualifier =
            selected ? nestedQualifier(importedType) : importedType.getQualifiedName().toString();
        int start = position(positions.getStartPosition(unit, identifier));
        int end = position(positions.getEndPosition(unit, identifier));
        if (start >= 0 && end > start) {
          edits.add(new Edit(start, end, qualifier + "." + simpleName));
          String packageName = trees.getElement(getCurrentPath()) == null
              ? ""
              : top.getEnclosingElement().toString();
          String currentPackage =
              unit.getPackageName() == null ? "" : unit.getPackageName().toString();
          if (selected
              && !packageName.equals("java.lang")
              && !packageName.equals(currentPackage)) {
            owners.add(top.getQualifiedName().toString());
          }
        }
        return super.visitIdentifier(identifier, unused);
      }

      @Override
      public Void visitMemberSelect(MemberSelectTree select, Void unused) {
        Element element = trees.getElement(getCurrentPath());
        if (!(element instanceof TypeElement type)) return super.visitMemberSelect(select, unused);
        TreePath parent = getCurrentPath().getParentPath();
        if (parent != null
            && parent.getLeaf() instanceof MemberSelectTree parentSelect
            && parentSelect.getExpression() == select
            && trees.getElement(parent) instanceof TypeElement) {
          return super.visitMemberSelect(select, unused);
        }
        int start = position(positions.getStartPosition(unit, select));
        int end = position(positions.getEndPosition(unit, select));
        if (start < 0 || end <= start) return super.visitMemberSelect(select, unused);
        if (!source.substring(start, end).equals(type.getQualifiedName().toString())) {
          return super.visitMemberSelect(select, unused);
        }
        TypeElement top = topLevelType(type);
        if (top == null) return super.visitMemberSelect(select, unused);
        if (!importChoices.selected(top.getQualifiedName().toString())) return null;
        edits.add(new Edit(start, end, nestedQualifier(type)));
        String packageName = top.getEnclosingElement().toString();
        String currentPackage =
            unit.getPackageName() == null ? "" : unit.getPackageName().toString();
        if (!packageName.equals("java.lang") && !packageName.equals(currentPackage)) {
          owners.add(top.getQualifiedName().toString());
        }
        return null;
      }
    }.scan(unit, null);
    return new StaticRewrite(edits, owners);
  }

  private static boolean staticMember(Element element) {
    return element.getModifiers().contains(Modifier.STATIC)
        || element.getKind() == ElementKind.ENUM_CONSTANT;
  }

  private static TypeElement enclosingType(Element element) {
    Element current = element;
    if (current instanceof TypeElement type) {
      current = type.getEnclosingElement();
    }
    while (current != null && !(current instanceof TypeElement)) {
      current = current.getEnclosingElement();
    }
    return current instanceof TypeElement type ? type : null;
  }

  private static TypeElement topLevelType(TypeElement type) {
    TypeElement current = type;
    while (current != null && current.getNestingKind() != NestingKind.TOP_LEVEL) {
      Element enclosing = current.getEnclosingElement();
      current = enclosing instanceof TypeElement parent ? parent : null;
    }
    return current;
  }

  private static String nestedQualifier(TypeElement type) {
    List<String> names = new ArrayList<>();
    TypeElement current = type;
    while (current != null) {
      names.add(current.getSimpleName().toString());
      if (current.getNestingKind() == NestingKind.TOP_LEVEL) break;
      Element enclosing = current.getEnclosingElement();
      current = enclosing instanceof TypeElement parent ? parent : null;
    }
    java.util.Collections.reverse(names);
    return String.join(".", names);
  }

  private static boolean insideCaseLabel(TreePath path) {
    TreePath current = path.getParentPath();
    while (current != null) {
      if (current.getLeaf() instanceof CaseTree) return true;
      if (current.getLeaf() instanceof MethodTree || current.getLeaf() instanceof ClassTree) return false;
      current = current.getParentPath();
    }
    return false;
  }

  private static String applyJjfsLayout(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    JavacTask task =
        (JavacTask)
            compiler.getTask(
                null,
                files,
                diagnostics,
                formatOptions(release, compilerOptions),
                null,
                List.of(new StringJavaFileObject(fileName, source)));
    try {
      CompilationUnitTree unit = task.parse().iterator().next();
      if (!convertErrors(diagnostics).isEmpty()) return source;
      Trees trees = Trees.instance(task);
      SourcePositions positions = trees.getSourcePositions();
      List<Edit> edits = new ArrayList<>();
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitMethod(MethodTree method, Void unused) {
          addParameterLayout(source, unit, method, positions, task, edits);
          addTypeParameterLayout(source, unit, method, positions, task, edits);
          addThrowsLayout(source, unit, method, positions, task, edits);
          return super.visitMethod(method, unused);
        }

        @Override
        public Void visitAnnotation(AnnotationTree annotation, Void unused) {
          addAnnotationLayout(
              source, unit, annotation, getCurrentPath(), positions, task, edits);
          return super.visitAnnotation(annotation, unused);
        }

        @Override
        public Void visitClass(ClassTree type, Void unused) {
          addTypeHeaderLayout(source, unit, type, positions, task, edits);
          return super.visitClass(type, unused);
        }

        @Override
        public Void visitMethodInvocation(MethodInvocationTree invocation, Void unused) {
          addInvocationLayout(source, unit, invocation, positions, task, edits);
          if (outermostChain(getCurrentPath(), invocation)) {
            addChainLayout(source, unit, invocation, positions, task, edits);
          }
          return super.visitMethodInvocation(invocation, unused);
        }

        @Override
        public Void visitIf(IfTree statement, Void unused) {
          addConditionLayout(source, unit, statement.getCondition(), statement, positions, task, edits);
          return super.visitIf(statement, unused);
        }

        @Override
        public Void visitWhileLoop(WhileLoopTree statement, Void unused) {
          addConditionLayout(source, unit, statement.getCondition(), statement, positions, task, edits);
          return super.visitWhileLoop(statement, unused);
        }

        @Override
        public Void visitDoWhileLoop(DoWhileLoopTree statement, Void unused) {
          addConditionLayout(source, unit, statement.getCondition(), statement, positions, task, edits);
          return super.visitDoWhileLoop(statement, unused);
        }

        @Override
        public Void visitForLoop(ForLoopTree statement, Void unused) {
          addForLayout(source, unit, statement, positions, task, edits);
          return super.visitForLoop(statement, unused);
        }

        @Override
        public Void visitEnhancedForLoop(EnhancedForLoopTree statement, Void unused) {
          addEnhancedForLayout(source, unit, statement, positions, task, edits);
          return super.visitEnhancedForLoop(statement, unused);
        }

        @Override
        public Void visitTry(TryTree statement, Void unused) {
          addResourceLayout(source, unit, statement, positions, task, edits);
          return super.visitTry(statement, unused);
        }

        @Override
        public Void visitNewArray(NewArrayTree array, Void unused) {
          addArrayLayout(source, unit, array, positions, task, edits);
          return super.visitNewArray(array, unused);
        }

        @Override
        public Void visitConditionalExpression(ConditionalExpressionTree expression, Void unused) {
          addTernaryLayout(source, unit, expression, positions, task, edits);
          return super.visitConditionalExpression(expression, unused);
        }

        @Override
        public Void visitLambdaExpression(LambdaExpressionTree lambda, Void unused) {
          removeOptionalLambdaParentheses(source, unit, lambda, positions, task, edits);
          return super.visitLambdaExpression(lambda, unused);
        }
      }.scan(unit, null);
      return applyEdits(source, edits);
    } catch (IOException exception) {
      throw new IllegalStateException("Unable to lay out Java source", exception);
    }
  }

  private static void addParameterLayout(
      String source,
      CompilationUnitTree unit,
      MethodTree method,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (method.getParameters().isEmpty()) return;
    int methodStart = position(positions.getStartPosition(unit, method));
    int firstStart = position(positions.getStartPosition(unit, method.getParameters().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, method.getParameters().get(method.getParameters().size() - 1)));
    int headerEnd =
        method.getBody() == null
            ? position(positions.getEndPosition(unit, method))
            : position(positions.getStartPosition(unit, method.getBody()));
    if (methodStart < 0 || firstStart < 0 || lastEnd < 0 || headerEnd < 0) return;
    boolean wrap =
        method.getParameters().size() > 3
            || compactWidth(source, methodStart, headerEnd) > 140;
    if (!wrap) return;

    int open = lastTokenPosition(source, task, methodStart, firstStart, "(");
    int close = firstTokenPosition(source, task, lastEnd, headerEnd, ")");
    if (open < 0 || close < 0) return;
    int indent = braceDepth(source, task, methodStart) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < method.getParameters().size(); index++) {
      int previousEnd =
          position(positions.getEndPosition(unit, method.getParameters().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, method.getParameters().get(index)));
      int comma = firstTokenPosition(source, task, previousEnd, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
  }

  private static void addInvocationLayout(
      String source,
      CompilationUnitTree unit,
      MethodInvocationTree invocation,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (invocation.getArguments().isEmpty()) return;
    int start = position(positions.getStartPosition(unit, invocation));
    int end = position(positions.getEndPosition(unit, invocation));
    int selectEnd = position(positions.getEndPosition(unit, invocation.getMethodSelect()));
    int firstStart = position(positions.getStartPosition(unit, invocation.getArguments().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, invocation.getArguments().get(invocation.getArguments().size() - 1)));
    if (start < 0 || end <= start || selectEnd < 0 || firstStart < 0 || lastEnd < 0) return;
    int open = firstTokenPosition(source, task, selectEnd, firstStart, "(");
    if (open < 0) return;
    long complexArguments =
        invocation.getArguments().stream().filter(JavaFormatter::complexArgument).count();
    int invocationWidth =
        invocation.getMethodSelect() instanceof MemberSelectTree select
            ? select.getIdentifier().length() + 1 + compactWidth(source, open, end)
            : compactWidth(source, start, end);
    if (invocationWidth <= 140 && complexArguments < 2) return;
    int close = lastTokenPosition(source, task, lastEnd, end, ")");
    if (close < 0) return;
    int indent = braceDepth(source, task, start) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < invocation.getArguments().size(); index++) {
      int previousEnd =
          position(positions.getEndPosition(unit, invocation.getArguments().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, invocation.getArguments().get(index)));
      int comma = firstTokenPosition(source, task, previousEnd, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
  }

  private static void addTypeParameterLayout(
      String source,
      CompilationUnitTree unit,
      MethodTree method,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (method.getTypeParameters().size() < 2) return;
    int methodStart = position(positions.getStartPosition(unit, method));
    int firstStart = position(positions.getStartPosition(unit, method.getTypeParameters().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, method.getTypeParameters().get(method.getTypeParameters().size() - 1)));
    int headerEnd =
        method.getBody() == null
            ? position(positions.getEndPosition(unit, method))
            : position(positions.getStartPosition(unit, method.getBody()));
    if (methodStart < 0
        || firstStart < 0
        || lastEnd <= firstStart
        || headerEnd <= methodStart
        || compactWidth(source, methodStart, headerEnd) <= 140) {
      return;
    }
    int open = lastTokenPosition(source, task, methodStart, firstStart, "<");
    int close = source.indexOf('>', lastEnd);
    if (close >= headerEnd) close = -1;
    if (open < 0 || close < 0) return;
    int indent = braceDepth(source, task, methodStart) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < method.getTypeParameters().size(); index++) {
      int previousStart =
          position(positions.getStartPosition(unit, method.getTypeParameters().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, method.getTypeParameters().get(index)));
      int comma = lastTokenPosition(source, task, previousStart, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
    if (method.getReturnType() != null) {
      int returnStart = position(positions.getStartPosition(unit, method.getReturnType()));
      if (returnStart > close) {
        edits.add(new Edit(close + 1, returnStart, "\n" + " ".repeat(indent)));
      }
    }
  }

  private static void addAnnotationLayout(
      String source,
      CompilationUnitTree unit,
      AnnotationTree annotation,
      TreePath annotationPath,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (annotation.getArguments().size() < 2) return;
    int start = position(positions.getStartPosition(unit, annotation));
    int end = position(positions.getEndPosition(unit, annotation));
    int firstStart = position(positions.getStartPosition(unit, annotation.getArguments().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, annotation.getArguments().get(annotation.getArguments().size() - 1)));
    if (start < 0 || end <= start || firstStart < 0 || lastEnd < firstStart) return;
    String authored = source.substring(start, end);
    if (!authored.contains("\n") && compactWidth(source, start, end) <= 80) return;
    int open = firstTokenPosition(source, task, start, firstStart, "(");
    int close = lastTokenPosition(source, task, lastEnd, end, ")");
    if (open < 0 || close < 0) return;
    int indent = lineIndent(source, start);
    TreePath ancestor = annotationPath.getParentPath();
    while (ancestor != null && !(ancestor.getLeaf() instanceof VariableTree)) {
      if (ancestor.getLeaf() instanceof MethodTree || ancestor.getLeaf() instanceof ClassTree) break;
      ancestor = ancestor.getParentPath();
    }
    if (ancestor != null && ancestor.getLeaf() instanceof VariableTree variable) {
      TreePath parent = ancestor.getParentPath();
      if (parent != null
          && parent.getLeaf() instanceof MethodTree method
          && method.getParameters().contains(variable)) {
        indent = braceDepth(source, task, start) * 2 + 2;
      }
    }
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < annotation.getArguments().size(); index++) {
      int previousStart =
          position(positions.getStartPosition(unit, annotation.getArguments().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, annotation.getArguments().get(index)));
      int comma = lastTokenPosition(source, task, previousStart, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
    if (ancestor != null && ancestor.getLeaf() instanceof VariableTree variable) {
      int typeStart = position(positions.getStartPosition(unit, variable.getType()));
      if (typeStart >= end) {
        edits.add(new Edit(end, typeStart, "\n" + " ".repeat(indent)));
      }
    }
  }

  private static int lineIndent(String source, int position) {
    int start = source.lastIndexOf('\n', Math.max(0, position - 1)) + 1;
    int cursor = start;
    while (cursor < position && source.charAt(cursor) == ' ') cursor++;
    return cursor - start;
  }

  private static void addThrowsLayout(
      String source,
      CompilationUnitTree unit,
      MethodTree method,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (method.getThrows().size() < 2) return;
    int methodStart = position(positions.getStartPosition(unit, method));
    int headerEnd =
        method.getBody() == null
            ? position(positions.getEndPosition(unit, method))
            : position(positions.getStartPosition(unit, method.getBody()));
    if (methodStart < 0 || headerEnd <= methodStart || compactWidth(source, methodStart, headerEnd) <= 140) {
      return;
    }
    int indent = braceDepth(source, task, methodStart) * 2 + 2;
    for (int index = 1; index < method.getThrows().size(); index++) {
      int previousStart = position(positions.getStartPosition(unit, method.getThrows().get(index - 1)));
      int currentStart = position(positions.getStartPosition(unit, method.getThrows().get(index)));
      int comma = lastTokenPosition(source, task, previousStart, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent)));
      }
    }
  }

  private static void addArrayLayout(
      String source,
      CompilationUnitTree unit,
      NewArrayTree array,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (array.getInitializers() == null) return;
    int start = position(positions.getStartPosition(unit, array));
    int end = position(positions.getEndPosition(unit, array));
    if (start < 0 || end <= start) return;
    if (array.getInitializers().isEmpty()) {
      int open = lastTokenPosition(source, task, start, end, "{");
      int close = lastTokenPosition(source, task, start, end, "}");
      if (open >= 0 && close > open) edits.add(new Edit(open, close + 1, "{}"));
      return;
    }
    int firstStart = position(positions.getStartPosition(unit, array.getInitializers().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, array.getInitializers().get(array.getInitializers().size() - 1)));
    int open = lastTokenPosition(source, task, start, firstStart, "{");
    int close = firstTokenPosition(source, task, lastEnd, end, "}");
    if (open < 0 || close < 0) return;
    boolean inline =
        compactWidth(source, open, close + 1) <= 140
            && array.getInitializers().stream().allMatch(JavaFormatter::simpleArrayElement)
            && !source.substring(open, close + 1).contains("//")
            && !source.substring(open, close + 1).contains("/*");
    if (inline) {
      List<String> elements =
          array.getInitializers().stream()
              .map(
                  element ->
                      source
                          .substring(
                              position(positions.getStartPosition(unit, element)),
                              position(positions.getEndPosition(unit, element)))
                          .replaceAll("\\s+", " "))
              .toList();
      edits.add(new Edit(open, close + 1, "{" + String.join(", ", elements) + "}"));
      return;
    }
    int indent = braceDepth(source, task, start) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < array.getInitializers().size(); index++) {
      int previousEnd =
          position(positions.getEndPosition(unit, array.getInitializers().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, array.getInitializers().get(index)));
      int comma = firstTokenPosition(source, task, previousEnd, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, ",\n" + " ".repeat(indent)));
  }

  private static boolean simpleArrayElement(Tree element) {
    return switch (element.getKind()) {
      case BOOLEAN_LITERAL,
          CHAR_LITERAL,
          DOUBLE_LITERAL,
          FLOAT_LITERAL,
          INT_LITERAL,
          LONG_LITERAL,
          NULL_LITERAL,
          STRING_LITERAL,
          IDENTIFIER,
          MEMBER_SELECT,
          UNARY_MINUS,
          UNARY_PLUS -> true;
      default -> false;
    };
  }

  private static void addTernaryLayout(
      String source,
      CompilationUnitTree unit,
      ConditionalExpressionTree expression,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    int start = position(positions.getStartPosition(unit, expression));
    int end = position(positions.getEndPosition(unit, expression));
    if (start < 0 || end <= start) return;
    boolean nested =
        expression.getTrueExpression() instanceof ConditionalExpressionTree
            || expression.getFalseExpression() instanceof ConditionalExpressionTree;
    if (!nested && compactWidth(source, start, end) <= 80) return;
    int conditionEnd = position(positions.getEndPosition(unit, expression.getCondition()));
    int trueStart = position(positions.getStartPosition(unit, expression.getTrueExpression()));
    int trueEnd = position(positions.getEndPosition(unit, expression.getTrueExpression()));
    int falseStart = position(positions.getStartPosition(unit, expression.getFalseExpression()));
    int question = firstTokenPosition(source, task, conditionEnd, trueStart, "?");
    int colon = firstTokenPosition(source, task, trueEnd, falseStart, ":");
    if (question < 0 || colon < 0) return;
    int indent = braceDepth(source, task, start) * 2 + 2;
    edits.add(new Edit(conditionEnd, question, "\n" + " ".repeat(indent)));
    edits.add(new Edit(trueEnd, colon, "\n" + " ".repeat(indent)));
  }

  private static void removeOptionalLambdaParentheses(
      String source,
      CompilationUnitTree unit,
      LambdaExpressionTree lambda,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (lambda.getParameters().size() != 1 || lambda.getParameters().get(0).getType() != null) {
      return;
    }
    int start = position(positions.getStartPosition(unit, lambda));
    int parameterStart = position(positions.getStartPosition(unit, lambda.getParameters().get(0)));
    int parameterEnd = position(positions.getEndPosition(unit, lambda.getParameters().get(0)));
    int bodyStart = position(positions.getStartPosition(unit, lambda.getBody()));
    if (start < 0 || parameterStart < 0 || parameterEnd < 0 || bodyStart < 0) return;
    int open = firstTokenPosition(source, task, start, parameterStart + 1, "(");
    int close = firstTokenPosition(source, task, parameterEnd, bodyStart, ")");
    if (open >= 0 && close >= 0) {
      edits.add(new Edit(close, close + 1, ""));
      edits.add(new Edit(open, open + 1, ""));
    }
  }

  private static boolean complexArgument(Tree argument) {
    if (argument instanceof LambdaExpressionTree || argument instanceof ConditionalExpressionTree) {
      return true;
    }
    if (argument instanceof MethodInvocationTree invocation) {
      return invocation.getArguments().stream().anyMatch(JavaFormatter::complexArgument);
    }
    return false;
  }

  private static void addConditionLayout(
      String source,
      CompilationUnitTree unit,
      Tree condition,
      Tree statement,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    while (condition instanceof ParenthesizedTree parenthesized) {
      condition = parenthesized.getExpression();
    }
    int conditionStart = position(positions.getStartPosition(unit, condition));
    int conditionEnd = position(positions.getEndPosition(unit, condition));
    int statementStart = position(positions.getStartPosition(unit, statement));
    if (conditionStart < 0 || conditionEnd <= conditionStart || statementStart < 0) return;
    List<Tree> operands = new ArrayList<>();
    List<String> operators = new ArrayList<>();
    collectLogicalOperands(condition, operands, operators);
    if (operands.size() < 2 || compactWidth(source, conditionStart, conditionEnd) <= 80) return;
    int open = lastTokenPosition(source, task, statementStart, conditionStart, "(");
    int close = firstTokenPosition(source, task, conditionEnd, position(positions.getEndPosition(unit, statement)), ")");
    if (open < 0 || close < 0) return;
    int indent = braceDepth(source, task, statementStart) * 2;
    int firstStart = position(positions.getStartPosition(unit, operands.get(0)));
    int lastEnd = position(positions.getEndPosition(unit, operands.get(operands.size() - 1)));
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < operands.size(); index++) {
      int previousEnd = position(positions.getEndPosition(unit, operands.get(index - 1)));
      int currentStart = position(positions.getStartPosition(unit, operands.get(index)));
      int operator = firstTokenPosition(source, task, previousEnd, currentStart, operators.get(index - 1));
      if (operator >= 0) {
        edits.add(new Edit(previousEnd, operator, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
  }

  private static void addForLayout(
      String source,
      CompilationUnitTree unit,
      ForLoopTree loop,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    int start = position(positions.getStartPosition(unit, loop));
    int bodyStart = position(positions.getStartPosition(unit, loop.getStatement()));
    if (start < 0 || bodyStart <= start || compactWidth(source, start, bodyStart) <= 140) return;
    int open = firstTokenPosition(source, task, start, bodyStart, "(");
    int close = lastTokenPosition(source, task, start, bodyStart, ")");
    if (open < 0 || close < 0) return;
    int indent = braceDepth(source, task, start) * 2;
    int first = firstTokenStartAfter(source, task, open + 1, close);
    if (first >= 0) edits.add(new Edit(open + 1, first, "\n" + " ".repeat(indent + 2)));
    List<Integer> semicolons = tokenPositions(source, task, open + 1, close, ";");
    for (int semicolon : semicolons) {
      int next = firstTokenStartAfter(source, task, semicolon + 1, close);
      if (next >= 0) {
        edits.add(new Edit(semicolon + 1, next, "\n" + " ".repeat(indent + 2)));
      }
    }
    int last = previousTokenEnd(source, task, open + 1, close);
    edits.add(new Edit(last, close, "\n" + " ".repeat(indent)));
  }

  private static void addEnhancedForLayout(
      String source,
      CompilationUnitTree unit,
      EnhancedForLoopTree loop,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    int start = position(positions.getStartPosition(unit, loop));
    int bodyStart = position(positions.getStartPosition(unit, loop.getStatement()));
    if (start < 0 || bodyStart <= start || compactWidth(source, start, bodyStart) <= 140) return;
    int variableStart = position(positions.getStartPosition(unit, loop.getVariable()));
    int variableEnd = position(positions.getEndPosition(unit, loop.getVariable()));
    int expressionStart = position(positions.getStartPosition(unit, loop.getExpression()));
    int expressionEnd = position(positions.getEndPosition(unit, loop.getExpression()));
    int open = firstTokenPosition(source, task, start, variableStart, "(");
    int colon = firstTokenPosition(source, task, variableEnd, expressionStart, ":");
    int close = firstTokenPosition(source, task, expressionEnd, bodyStart, ")");
    if (open < 0 || colon < 0 || close < 0) return;
    int indent = braceDepth(source, task, start) * 2;
    edits.add(new Edit(open + 1, variableStart, "\n" + " ".repeat(indent + 2)));
    edits.add(new Edit(colon + 1, expressionStart, "\n" + " ".repeat(indent + 2)));
    edits.add(new Edit(expressionEnd, close, "\n" + " ".repeat(indent)));
  }

  private static void addResourceLayout(
      String source,
      CompilationUnitTree unit,
      TryTree statement,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (statement.getResources().isEmpty()) return;
    int start = position(positions.getStartPosition(unit, statement));
    int bodyStart = position(positions.getStartPosition(unit, statement.getBlock()));
    if (start < 0 || bodyStart <= start) return;
    if (statement.getResources().size() == 1 && compactWidth(source, start, bodyStart) <= 140) {
      return;
    }
    int firstStart = position(positions.getStartPosition(unit, statement.getResources().get(0)));
    int lastEnd =
        position(
            positions.getEndPosition(
                unit, statement.getResources().get(statement.getResources().size() - 1)));
    int open = firstTokenPosition(source, task, start, firstStart, "(");
    int close = firstTokenPosition(source, task, lastEnd, bodyStart, ")");
    if (open < 0 || close < 0) return;
    int indent = braceDepth(source, task, start) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < statement.getResources().size(); index++) {
      int previousStart =
          position(positions.getStartPosition(unit, statement.getResources().get(index - 1)));
      int currentStart =
          position(positions.getStartPosition(unit, statement.getResources().get(index)));
      int semicolon = lastTokenPosition(source, task, previousStart, currentStart, ";");
      if (semicolon >= 0) {
        edits.add(new Edit(semicolon + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
  }

  private static void collectLogicalOperands(
      Tree expression, List<Tree> operands, List<String> operators) {
    if (expression instanceof BinaryTree binary
        && (binary.getKind() == Tree.Kind.CONDITIONAL_AND
            || binary.getKind() == Tree.Kind.CONDITIONAL_OR)) {
      collectLogicalOperands(binary.getLeftOperand(), operands, operators);
      operators.add(binary.getKind() == Tree.Kind.CONDITIONAL_AND ? "&&" : "||");
      collectLogicalOperands(binary.getRightOperand(), operands, operators);
      return;
    }
    operands.add(expression);
  }

  private static void addTypeHeaderLayout(
      String source,
      CompilationUnitTree unit,
      ClassTree type,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    int start = position(positions.getStartPosition(unit, type));
    int end = position(positions.getEndPosition(unit, type));
    if (start < 0 || end <= start) return;
    int openBrace = typeBodyOpen(source, task, start, end);
    addRecordComponentLayout(source, unit, type, openBrace, positions, task, edits);
    if (openBrace < 0 || compactWidth(source, start, openBrace) <= 140) return;
    int indent = braceDepth(source, task, start) * 2;

    int previousClauseEnd = -1;
    if (type.getExtendsClause() != null) {
      int clauseStart = position(positions.getStartPosition(unit, type.getExtendsClause()));
      int keyword = lastTokenPosition(source, task, start, clauseStart, "extends");
      if (keyword >= 0) {
        int previousEnd = previousTokenEnd(source, task, start, keyword);
        edits.add(new Edit(previousEnd, keyword, "\n" + " ".repeat(indent + 2)));
      }
      previousClauseEnd = position(positions.getEndPosition(unit, type.getExtendsClause()));
    }

    List<? extends Tree> implemented = type.getImplementsClause();
    if (!implemented.isEmpty()) {
      int firstStart = position(positions.getStartPosition(unit, implemented.get(0)));
      int keyword = lastTokenPosition(source, task, Math.max(start, previousClauseEnd), firstStart, "implements");
      if (keyword < 0) {
        keyword = lastTokenPosition(source, task, Math.max(start, previousClauseEnd), firstStart, "extends");
      }
      if (keyword >= 0) {
        int previousEnd = previousTokenEnd(source, task, start, keyword);
        edits.add(new Edit(previousEnd, keyword, "\n" + " ".repeat(indent + 2)));
      }
      addClauseContinuations(source, unit, implemented, positions, task, indent + 4, edits);
      previousClauseEnd = position(positions.getEndPosition(unit, implemented.get(implemented.size() - 1)));
    }

    List<? extends Tree> permitted = type.getPermitsClause();
    if (!permitted.isEmpty()) {
      int firstStart = position(positions.getStartPosition(unit, permitted.get(0)));
      int keyword = lastTokenPosition(source, task, Math.max(start, previousClauseEnd), firstStart, "permits");
      if (keyword >= 0) {
        int previousEnd = previousTokenEnd(source, task, start, keyword);
        edits.add(new Edit(previousEnd, keyword, "\n" + " ".repeat(indent + 2)));
      }
      addClauseContinuations(source, unit, permitted, positions, task, indent + 4, edits);
      previousClauseEnd = position(positions.getEndPosition(unit, permitted.get(permitted.size() - 1)));
    }

    int headerEnd = previousClauseEnd >= 0 ? previousClauseEnd : previousTokenEnd(source, task, start, openBrace);
    if (headerEnd >= 0) edits.add(new Edit(headerEnd, openBrace, "\n" + " ".repeat(indent)));
  }

  private static void addRecordComponentLayout(
      String source,
      CompilationUnitTree unit,
      ClassTree type,
      int openBrace,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    if (type.getKind() != Tree.Kind.RECORD || openBrace < 0) return;
    List<VariableTree> components =
        type.getMembers().stream()
            .filter(VariableTree.class::isInstance)
            .map(VariableTree.class::cast)
            .filter(component -> position(positions.getStartPosition(unit, component)) < openBrace)
            .toList();
    if (components.isEmpty()) return;
    int firstStart = position(positions.getStartPosition(unit, components.get(0)));
    int lastEnd = position(positions.getEndPosition(unit, components.get(components.size() - 1)));
    if (firstStart < 0 || lastEnd <= firstStart) return;
    int open = lastTokenPosition(source, task, position(positions.getStartPosition(unit, type)), firstStart, "(");
    int close = firstTokenPosition(source, task, lastEnd, openBrace, ")");
    if (open < 0 || close < 0) return;
    if (components.size() <= 3 && compactWidth(source, open, close + 1) <= 140) return;
    int indent = braceDepth(source, task, open) * 2;
    edits.add(new Edit(open + 1, firstStart, "\n" + " ".repeat(indent + 2)));
    for (int index = 1; index < components.size(); index++) {
      int previousStart = position(positions.getStartPosition(unit, components.get(index - 1)));
      int currentStart = position(positions.getStartPosition(unit, components.get(index)));
      int comma = lastTokenPosition(source, task, previousStart, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent + 2)));
      }
    }
    edits.add(new Edit(lastEnd, close, "\n" + " ".repeat(indent)));
  }

  private static int typeBodyOpen(String source, JavacTask task, int start, int end) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    int parentheses = 0;
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) return -1;
      if (token.pos < start) continue;
      String text = source.substring(token.pos, token.endPos);
      if (text.equals("(")) parentheses++;
      else if (text.equals(")")) parentheses = Math.max(0, parentheses - 1);
      else if (text.equals("{") && parentheses == 0) return token.pos;
    }
  }

  private static void addClauseContinuations(
      String source,
      CompilationUnitTree unit,
      List<? extends Tree> types,
      SourcePositions positions,
      JavacTask task,
      int indent,
      List<Edit> edits) {
    for (int index = 1; index < types.size(); index++) {
      int previousEnd = position(positions.getEndPosition(unit, types.get(index - 1)));
      int currentStart = position(positions.getStartPosition(unit, types.get(index)));
      int comma = firstTokenPosition(source, task, previousEnd, currentStart, ",");
      if (comma >= 0) {
        edits.add(new Edit(comma + 1, currentStart, "\n" + " ".repeat(indent)));
      }
    }
  }

  private static boolean outermostChain(TreePath path, MethodInvocationTree invocation) {
    TreePath parentPath = path.getParentPath();
    if (parentPath == null || !(parentPath.getLeaf() instanceof MemberSelectTree select)) {
      return true;
    }
    if (select.getExpression() != invocation) return true;
    TreePath grandparent = parentPath.getParentPath();
    return grandparent == null
        || !(grandparent.getLeaf() instanceof MethodInvocationTree outer)
        || outer.getMethodSelect() != select;
  }

  private static void addChainLayout(
      String source,
      CompilationUnitTree unit,
      MethodInvocationTree outer,
      SourcePositions positions,
      JavacTask task,
      List<Edit> edits) {
    List<MethodInvocationTree> calls = new ArrayList<>();
    List<MemberSelectTree> continuationSelects = new ArrayList<>();
    collectChain(outer, calls, continuationSelects);
    if (calls.size() < 2) return;
    int start = position(positions.getStartPosition(unit, outer));
    int end = position(positions.getEndPosition(unit, outer));
    if (start < 0 || end <= start) return;
    boolean complex = calls.stream().anyMatch(JavaFormatter::complexInvocation);
    boolean blockLambda = calls.stream().anyMatch(JavaFormatter::containsBlockLambda);
    int width = compactWidth(source, start, end);
    boolean wrap =
        width > 140
            || calls.size() >= 5
            || blockLambda
            || (complex && width > 80)
            || (calls.size() >= 3 && complex)
            || (calls.size() >= 3 && width > 100);
    if (!wrap) return;
    int indent = braceDepth(source, task, start) * 2 + 2;
    for (MemberSelectTree select : continuationSelects) {
      int expressionEnd = position(positions.getEndPosition(unit, select.getExpression()));
      int selectEnd = position(positions.getEndPosition(unit, select));
      int dot = firstTokenPosition(source, task, expressionEnd, selectEnd, ".");
      if (dot >= 0) edits.add(new Edit(dot, dot, "\n" + " ".repeat(indent)));
    }
  }

  private static void collectChain(
      MethodInvocationTree invocation,
      List<MethodInvocationTree> calls,
      List<MemberSelectTree> continuationSelects) {
    if (invocation.getMethodSelect() instanceof MemberSelectTree select
        && select.getExpression() instanceof MethodInvocationTree previous) {
      collectChain(previous, calls, continuationSelects);
      calls.add(invocation);
      continuationSelects.add(select);
      return;
    }
    calls.add(invocation);
  }

  private static boolean complexInvocation(MethodInvocationTree invocation) {
    for (Tree argument : invocation.getArguments()) {
      if (argument instanceof LambdaExpressionTree
          || argument instanceof ConditionalExpressionTree
          || argument instanceof MethodInvocationTree) return true;
    }
    return false;
  }

  private static boolean containsBlockLambda(MethodInvocationTree invocation) {
    return invocation.getArguments().stream()
        .filter(LambdaExpressionTree.class::isInstance)
        .map(LambdaExpressionTree.class::cast)
        .anyMatch(lambda -> lambda.getBodyKind() == LambdaExpressionTree.BodyKind.STATEMENT);
  }

  private static int compactWidth(String source, int start, int end) {
    return source.substring(start, end).replaceAll("\\s+", " ").length();
  }

  private static int position(long position) {
    return position < 0 || position > Integer.MAX_VALUE ? -1 : (int) position;
  }

  private static int firstTokenPosition(
      String source, JavacTask task, int start, int end, String expected) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) return -1;
      if (token.pos >= start
          && token.endPos <= end
          && source.substring(token.pos, token.endPos).equals(expected)) {
        return token.pos;
      }
    }
  }

  private static int firstTokenStartAfter(
      String source, JavacTask task, int start, int end) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) return -1;
      if (token.pos >= start) return token.pos;
    }
  }

  private static List<Integer> tokenPositions(
      String source, JavacTask task, int start, int end, String expected) {
    List<Integer> positions = new ArrayList<>();
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) return positions;
      if (token.pos >= start && source.substring(token.pos, token.endPos).equals(expected)) {
        positions.add(token.pos);
      }
    }
  }

  private static int braceDepth(String source, JavacTask task, int position) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    int depth = 0;
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= position) return depth;
      String text = source.substring(token.pos, token.endPos);
      if (text.equals("{")) depth++;
      else if (text.equals("}")) depth = Math.max(0, depth - 1);
    }
  }

  private static int previousTokenEnd(String source, JavacTask task, int start, int end) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    int previousEnd = start;
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) return previousEnd;
      if (token.pos >= start) previousEnd = token.endPos;
    }
  }

  private static Edit orderedMembers(
      String source,
      CompilationUnitTree unit,
      ClassTree type,
      SourcePositions positions,
      JavacTask task,
      Trees trees) {
    // Enum constants are not ordinary field declarations: commas and the
    // optional body semicolon belong to the enum grammar rather than to each
    // VariableTree source range. Their authored order is semantic.
    if (type.getKind() == Tree.Kind.ENUM) return null;
    List<MemberSource> members = new ArrayList<>();
    int declarationStart = Math.toIntExact(positions.getStartPosition(unit, type));
    int declarationEnd = Math.toIntExact(positions.getEndPosition(unit, type));
    if (declarationStart < 0 || declarationEnd <= declarationStart) return null;
    if (source.substring(declarationStart, declarationEnd).contains("// jjfs: off")) return null;

    int previousEnd = -1;
    int index = 0;
    for (Tree member : type.getMembers()) {
      long rawStart = positions.getStartPosition(unit, member);
      long rawEnd = positions.getEndPosition(unit, member);
      if (rawStart < 0 || rawEnd <= rawStart) continue;
      int start = Math.toIntExact(rawStart);
      int end = Math.toIntExact(rawEnd);
      if (previousEnd > start) return null;
      members.add(new MemberSource(member, start, end, index++));
      previousEnd = end;
    }
    if (members.size() < 2) return null;

    int openBrace =
        lastTokenPosition(source, task, declarationStart, members.get(0).start(), "{");
    int closeBrace =
        lastTokenPosition(
            source, task, members.get(members.size() - 1).end(), declarationEnd, "}");
    if (openBrace < declarationStart || closeBrace < members.get(members.size() - 1).end()) {
      return null;
    }

    int cursor = openBrace + 1;
    String prefix = "";
    int firstLineEnd = source.indexOf('\n', openBrace + 1);
    if (firstLineEnd >= 0 && firstLineEnd < members.get(0).start()) {
      String headerSuffix = source.substring(openBrace + 1, firstLineEnd);
      if (!headerSuffix.isBlank()) {
        prefix = headerSuffix.stripTrailing();
        cursor = firstLineEnd + 1;
      }
    }
    Map<String, Integer> overloadPositions = new HashMap<>();
    for (MemberSource member : members) {
      if (member.tree() instanceof MethodTree method && method.getReturnType() != null) {
        int category = memberCategory(unit, type, method, trees, task);
        overloadPositions.putIfAbsent(category + ":" + method.getName(), member.index());
      }
    }
    List<MemberChunk> chunks = new ArrayList<>();
    for (int memberIndex = 0; memberIndex < members.size(); memberIndex++) {
      MemberSource member = members.get(memberIndex);
      int boundary =
          memberIndex + 1 < members.size() ? members.get(memberIndex + 1).start() : closeBrace;
      int chunkEnd = member.end();
      int lineEnd = source.indexOf('\n', member.end());
      int trailingEnd = lineEnd < 0 ? boundary : Math.min(lineEnd, boundary);
      if (trailingEnd > member.end()
          && !source.substring(member.end(), trailingEnd).isBlank()) {
        chunkEnd = trailingEnd;
      }
      String chunk = source.substring(cursor, chunkEnd).strip();
      if (member.tree() instanceof ClassTree nested) {
        Edit nestedEdit = orderedMembers(source, unit, nested, positions, task, trees);
        if (nestedEdit != null
            && nestedEdit.start() >= cursor
            && nestedEdit.end() <= member.end()) {
          int relativeStart = nestedEdit.start() - cursor;
          int relativeEnd = nestedEdit.end() - cursor;
          String raw = source.substring(cursor, member.end());
          chunk =
              (raw.substring(0, relativeStart)
                      + nestedEdit.replacement()
                      + raw.substring(relativeEnd))
                  .strip();
        }
      }
      int category = memberCategory(unit, type, member.tree(), trees, task);
      int sortIndex = member.index();
      if (member.tree() instanceof MethodTree method && method.getReturnType() != null) {
        sortIndex = overloadPositions.get(category + ":" + method.getName());
      }
      chunks.add(
          new MemberChunk(
              category,
              memberGroup(unit, member.tree(), trees),
              sortIndex,
              member.index(),
              chunk));
      cursor = chunkEnd;
    }
    chunks.sort(
        Comparator.comparingInt(MemberChunk::category)
            .thenComparingInt(MemberChunk::sortIndex)
            .thenComparingInt(MemberChunk::originalIndex));
    StringBuilder ordered = new StringBuilder();
    MemberChunk previous = null;
    for (MemberChunk chunk : chunks) {
      if (previous != null) {
        ordered.append(previous.group() == chunk.group() && chunk.group() > 0 ? '\n' : "\n\n");
      }
      ordered.append(chunk.source());
      previous = chunk;
    }
    String replacement =
        prefix.isEmpty() ? "\n" + ordered + "\n" : prefix + "\n" + ordered + "\n";
    return new Edit(openBrace + 1, closeBrace, replacement);
  }

  private static int lastTokenPosition(
      String source, JavacTask task, int start, int end, String expected) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, false);
    int position = -1;
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.kind == Tokens.TokenKind.EOF || token.pos >= end) break;
      if (token.pos >= start
          && token.endPos <= end
          && source.substring(token.pos, token.endPos).equals(expected)) {
        position = token.pos;
      }
    }
    return position;
  }

  private static int memberCategory(
      CompilationUnitTree unit,
      ClassTree enclosing,
      Tree tree,
      Trees trees,
      JavacTask task) {
    if (tree instanceof VariableTree variable) {
      Element element = trees.getElement(com.sun.source.util.TreePath.getPath(unit, variable));
      if (element instanceof VariableElement field
          && field.getModifiers().contains(Modifier.STATIC)
          && field.getModifiers().contains(Modifier.FINAL)
          && field.getConstantValue() != null) {
        return 10 + visibility(field.getModifiers());
      }
      return variable.getModifiers().getFlags().contains(Modifier.STATIC) ? 20 : 21;
    }
    if (tree instanceof BlockTree block) return block.isStatic() ? 20 : 21;
    if (tree instanceof MethodTree method) {
      if (method.getReturnType() == null) return 30;
      Element element = trees.getElement(com.sun.source.util.TreePath.getPath(unit, method));
      Element enclosingElement =
          trees.getElement(com.sun.source.util.TreePath.getPath(unit, enclosing));
      if (element instanceof ExecutableElement executable
          && enclosingElement instanceof TypeElement type
          && executable.getModifiers().contains(Modifier.STATIC)
          && task.getTypes().isAssignable(executable.getReturnType(), type.asType())) {
        return 35;
      }
      return 40 + visibility(method.getModifiers().getFlags());
    }
    if (tree instanceof ClassTree nested) {
      return 50 + visibility(nested.getModifiers().getFlags());
    }
    return 60;
  }

  private static int memberGroup(CompilationUnitTree unit, Tree tree, Trees trees) {
    if (tree instanceof VariableTree variable) {
      Element element = trees.getElement(com.sun.source.util.TreePath.getPath(unit, variable));
      if (element instanceof VariableElement field
          && field.getModifiers().contains(Modifier.STATIC)
          && field.getModifiers().contains(Modifier.FINAL)
          && field.getConstantValue() != null) {
        return 1;
      }
      return variable.getModifiers().getFlags().contains(Modifier.STATIC) ? 2 : 3;
    }
    if (tree instanceof BlockTree block) return block.isStatic() ? 2 : 3;
    return 0;
  }

  private static int visibility(Set<Modifier> modifiers) {
    if (modifiers.contains(Modifier.PUBLIC)) return 0;
    if (modifiers.contains(Modifier.PROTECTED)) return 1;
    if (modifiers.contains(Modifier.PRIVATE)) return 3;
    return 2;
  }

  private static String formatTokens(String source, JavacTask task) {
    Lexer scanner =
        ScannerFactory.instance(((BasicJavacTask) task).getContext()).newScanner(source, true);
    List<Lexeme> lexemes = new ArrayList<>();
    Set<String> comments = new HashSet<>();
    while (true) {
      scanner.nextToken();
      Tokens.Token token = scanner.token();
      if (token.comments != null) {
        for (Tokens.Comment comment : token.comments) {
          int start = comment.getPos().getStartPosition();
          int end = comment.getPos().getEndPosition(null);
          if (start >= 0 && end > start && end <= source.length()) {
            String key = start + ":" + end;
            if (comments.add(key)) {
              lexemes.add(
                  new Lexeme(
                      source.substring(start, end),
                      start,
                      end,
                      true,
                      comment.getStyle().name()));
            }
          }
        }
      }
      if (token.kind == Tokens.TokenKind.EOF) break;
      lexemes.add(
          new Lexeme(
              source.substring(token.pos, token.endPos),
              token.pos,
              token.endPos,
              false,
              token.kind.name()));
    }
    lexemes.sort(Comparator.comparingInt(Lexeme::start).thenComparingInt(Lexeme::end));
    return new Renderer(source, lexemes).render();
  }

  private static final class Renderer {
    private final String source;
    private final List<Lexeme> lexemes;
    private final StringBuilder output = new StringBuilder();
    private int indent;
    private int parenthesisDepth;
    private int genericDepth;
    private int ternaryDepth;
    private boolean previousGenericClose;
    private boolean previousMethodTypeArgumentClose;
    private final java.util.ArrayDeque<Boolean> genericMethodArguments =
        new java.util.ArrayDeque<>();
    private int previousEnd;
    private boolean lineStart = true;
    private int enumConstantDepth = -1;

    Renderer(String source, List<Lexeme> lexemes) {
      this.source = source;
      this.lexemes = lexemes;
    }

    String render() {
      Lexeme previous = null;
      for (int index = 0; index < lexemes.size(); index++) {
        Lexeme current = lexemes.get(index);
        String gap = source.substring(Math.min(previousEnd, current.start()), current.start());
        if (current.comment() && current.text().strip().equals("// jjfs: off")) {
          comment(current, gap);
          int enabled = matchingEnableDirective(index + 1);
          if (enabled < 0) {
            previousEnd = current.end();
            previous = current;
            continue;
          }
          Lexeme on = lexemes.get(enabled);
          int contentStart = lineContentStartAfter(current.end());
          int contentEnd = lineStart(on.start());
          if (contentEnd > contentStart) {
            output.append(source, contentStart, contentEnd);
            lineStart = output.isEmpty() || output.charAt(output.length() - 1) == '\n';
          }
          append(on.text());
          newline(false);
          previousEnd = on.end();
          previous = on;
          index = enabled;
          continue;
        }
        if (current.comment()) {
          comment(current, gap);
        } else {
          token(index, previous, current, gap);
          previous = current;
        }
        previousEnd = current.end();
      }
      trimTrailingSpace();
      if (!lineStart) newline(false);
      while (output.length() > 1 && output.charAt(output.length() - 2) == '\n') {
        output.deleteCharAt(output.length() - 1);
      }
      return output.toString();
    }

    private int matchingEnableDirective(int start) {
      for (int index = start; index < lexemes.size(); index++) {
        Lexeme candidate = lexemes.get(index);
        if (candidate.comment() && candidate.text().strip().equals("// jjfs: on")) return index;
      }
      return -1;
    }

    private int lineContentStartAfter(int position) {
      int cursor = position;
      if (cursor < source.length() && source.charAt(cursor) == '\r') cursor++;
      if (cursor < source.length() && source.charAt(cursor) == '\n') cursor++;
      return cursor;
    }

    private int lineStart(int position) {
      int newline = source.lastIndexOf('\n', Math.max(0, position - 1));
      return newline < 0 ? 0 : newline + 1;
    }

    private void comment(Lexeme comment, String gap) {
      boolean lineComment = comment.kind().equals("LINE") || comment.kind().equals("JAVADOC_LINE");
      boolean trailing = !lineStart && !gap.contains("\n");
      if (!lineStart) {
        if (gap.contains("\n")) newline(blankGap(gap));
        else space();
      }
      append(JjfsComments.format(comment.text(), comment.kind(), indent, trailing));
      if (lineComment || gap.contains("\n")) newline(false);
    }

    private void token(int index, Lexeme previous, Lexeme current, String gap) {
      String text = current.text();
      String before = previous == null ? "" : previous.text();
      if (enumConstantDepth >= 0
          && indent == enumConstantDepth
          && parenthesisDepth == 0
          && text.equals(";")
          && !before.equals(",")) {
        append(",");
        newline(false);
      }
      boolean closingBrace = text.equals("}");
      if (closingBrace) {
        if (enumConstantDepth >= 0
            && indent == enumConstantDepth
            && parenthesisDepth == 0
            && !before.equals(";")
            && !before.equals(",")) {
          append(",");
        }
        indent = Math.max(0, indent - 1);
        if (!lineStart) newline(false);
      } else if (gap.contains("\n") && immediatelyFollowsDocumentationComment(index)) {
        newline(false);
      } else if (previous == null && !lineStart && gap.contains("\n")) {
        newline(blankGap(gap));
      } else if (previous != null) {
        if (gap.contains("\n") && previousAnnotationEnds(index)) {
          newline(false);
        } else if (before.equals("{")) {
          newline(false);
        } else if (before.equals(";") && parenthesisDepth == 0) {
          newline(blankGap(gap));
        } else if (before.equals("}") && !Set.of("else", "catch", "finally", "while").contains(text)
            && !Set.of(";", ",", ")").contains(text)) {
          newline(indent == 0 || blankGap(gap));
        } else if (blankGap(gap) && lineStart) {
          newline(true);
        } else if (needsSpace(index, before, text)) {
          space();
        }
      }

      append(text);
      if (text.equals("{")) {
        indent++;
        if (precededByKeyword(index, "enum")) enumConstantDepth = indent;
      }
      if (text.equals(",")
          && enumConstantDepth >= 0
          && indent == enumConstantDepth
          && parenthesisDepth == 0) {
        newline(false);
      }
      if (text.equals(";") && enumConstantDepth == indent && parenthesisDepth == 0) {
        enumConstantDepth = -1;
      }
      if (text.equals("(")) parenthesisDepth++;
      if (text.equals(")")) parenthesisDepth = Math.max(0, parenthesisDepth - 1);
      if (text.equals("<") && genericOpening(index)) {
        genericDepth++;
        genericMethodArguments.push(precededBy(index, Set.of(".", "::")));
      }
      boolean genericClose = text.chars().allMatch(character -> character == '>') && genericDepth > 0;
      boolean methodTypeArgumentClose = false;
      if (genericClose) {
        for (int closed = 0; closed < text.length() && !genericMethodArguments.isEmpty(); closed++) {
          methodTypeArgumentClose |= genericMethodArguments.pop();
        }
        genericDepth = Math.max(0, genericDepth - text.length());
      }
      if (text.equals("?") && genericDepth == 0) ternaryDepth++;
      if (text.equals(":") && ternaryDepth > 0) ternaryDepth--;
      previousGenericClose = genericClose;
      previousMethodTypeArgumentClose = methodTypeArgumentClose;
    }

    private boolean immediatelyFollowsDocumentationComment(int index) {
      if (index <= 0) return false;
      Lexeme preceding = lexemes.get(index - 1);
      return preceding.comment()
          && (preceding.kind().startsWith("JAVADOC") || preceding.text().startsWith("/**"));
    }

    private boolean needsSpace(int index, String previous, String current) {
      if (Set.of(")", "]", ",", ";", ".", "::").contains(current)) return false;
      if (Set.of("(", "[", ".", "::", "@").contains(previous)) return false;
      if (previousAnnotationEnds(index) && word(current)) return true;
      if (current.equals("(")) {
        if (previousGenericClose) return false;
        return CONTROL_PARENTHESIS.contains(previous)
            || Set.of("return", "throw", "assert", "yield").contains(previous)
            || BINARY_OPERATORS.contains(previous);
      }
      if (current.equals("{") || Set.of("else", "catch", "finally").contains(current)) return true;
      if (previous.equals(",")) return true;
      if (previous.equals(";") && parenthesisDepth > 0) return true;
      if (previous.equals(")") && current.equals("throws")) return true;
      if (previous.equals("]") && word(current)) return true;
      if (previous.equals("...")) return true;
      if (current.equals("?") && genericDepth == 0) return true;
      if (previous.equals("?") && genericDepth == 0) return true;
      if (current.equals(":")) return ternaryDepth > 0 || parenthesisDepth > 0;
      if (previous.equals(":")) return true;
      if (previous.equals("?") && Set.of("extends", "super").contains(current)) return true;
      if (current.equals("<") && genericOpening(index)) {
        return Set.of(
                "public",
                "protected",
                "private",
                "abstract",
                "default",
                "static",
                "final",
                "synchronized",
                "native",
                "strictfp")
            .contains(previous);
      }
      if (genericDepth > 0 && (current.chars().allMatch(c -> c == '>') || previous.equals("<"))) {
        return false;
      }
      if (Set.of("+", "-").contains(current) && unaryPlusOrMinus(index)) return true;
      if (Set.of("+", "-").contains(previous) && unaryPlusOrMinus(index - 1)) return false;
      if (previousGenericClose && previousMethodTypeArgumentClose && word(current)) return false;
      if (BINARY_OPERATORS.contains(current) || BINARY_OPERATORS.contains(previous)) return true;
      if (Set.of("!", "~", "++", "--").contains(previous)
          || Set.of("++", "--").contains(current)) return false;
      return word(previous) && word(current);
    }

    private boolean precededBy(int index, Set<String> values) {
      for (int previous = index - 1; previous >= 0; previous--) {
        Lexeme candidate = lexemes.get(previous);
        if (!candidate.comment()) return values.contains(candidate.text());
      }
      return false;
    }

    private boolean precededByKeyword(int index, String keyword) {
      for (int previous = index - 1; previous >= 0; previous--) {
        Lexeme candidate = lexemes.get(previous);
        if (candidate.comment()) continue;
        String text = candidate.text();
        if (text.equals(keyword)) return true;
        if (Set.of("{", "}", ";").contains(text)) return false;
      }
      return false;
    }

    private boolean previousAnnotationEnds(int index) {
      int previous = previousCodeIndex(index - 1);
      if (previous < 0) return false;
      if (lexemes.get(previous).text().equals(")")) {
        int depth = 0;
        for (int cursor = previous; cursor >= 0; cursor--) {
          Lexeme candidate = lexemes.get(cursor);
          if (candidate.comment()) continue;
          if (candidate.text().equals(")")) depth++;
          else if (candidate.text().equals("(")) {
            depth--;
            if (depth == 0) {
              int name = previousCodeIndex(cursor - 1);
              int marker = previousCodeIndex(name - 1);
              return name >= 0 && marker >= 0 && lexemes.get(marker).text().equals("@");
            }
          }
        }
        return false;
      }
      int marker = previousCodeIndex(previous - 1);
      return marker >= 0 && lexemes.get(marker).text().equals("@");
    }

    private int previousCodeIndex(int index) {
      for (int cursor = index; cursor >= 0; cursor--) {
        if (!lexemes.get(cursor).comment()) return cursor;
      }
      return -1;
    }

    private boolean unaryPlusOrMinus(int index) {
      if (index < 0 || index >= lexemes.size()) return false;
      String operator = lexemes.get(index).text();
      if (!operator.equals("+") && !operator.equals("-")) return false;
      for (int previous = index - 1; previous >= 0; previous--) {
        Lexeme candidate = lexemes.get(previous);
        if (candidate.comment()) continue;
        String text = candidate.text();
        return Set.of("(", "[", "{", ",", ";", "=", ":", "?", "->", "return", "case", "yield")
                .contains(text)
            || BINARY_OPERATORS.contains(text);
      }
      return true;
    }

    private boolean genericOpening(int index) {
      if (!lexemes.get(index).text().equals("<")) return false;
      int depth = 0;
      for (int next = index + 1; next < lexemes.size(); next++) {
        Lexeme candidate = lexemes.get(next);
        if (candidate.comment()) continue;
        String text = candidate.text();
        if (text.equals("<")) depth++;
        else if (text.chars().allMatch(character -> character == '>')) {
          if (depth == 0) return true;
          depth -= text.length();
          if (depth < 0) return true;
        } else if (Set.of(";", "{", "}", "=", "==", "&&", "||").contains(text)) {
          return false;
        }
      }
      return false;
    }

    private static boolean word(String value) {
      if (value.isEmpty()) return false;
      int first = value.codePointAt(0);
      int last = value.codePointBefore(value.length());
      return (Character.isJavaIdentifierPart(first) || Character.isDigit(first) || first == '"' || first == '\'')
          && (Character.isJavaIdentifierPart(last) || Character.isDigit(last) || last == '"' || last == '\'');
    }

    private void append(String value) {
      if (lineStart) {
        output.append(INDENT.repeat(indent));
      }
      output.append(value);
      lineStart = value.endsWith("\n");
    }

    private void space() {
      if (!lineStart && !output.isEmpty() && output.charAt(output.length() - 1) != ' ') output.append(' ');
    }

    private void newline(boolean blank) {
      trimTrailingSpace();
      if (!output.isEmpty() && output.charAt(output.length() - 1) != '\n') output.append('\n');
      if (blank && !output.isEmpty() && (output.length() < 2 || output.charAt(output.length() - 2) != '\n')) {
        output.append('\n');
      }
      lineStart = true;
    }

    private void trimTrailingSpace() {
      while (!output.isEmpty() && output.charAt(output.length() - 1) == ' ') {
        output.deleteCharAt(output.length() - 1);
      }
    }

    private static boolean blankGap(String gap) {
      return gap.indexOf('\n') >= 0 && gap.indexOf('\n') != gap.lastIndexOf('\n');
    }
  }

  private record Edit(int start, int end, String replacement) {}

  private record MemberSource(Tree tree, int start, int end, int index) {}

  private record MemberChunk(
      int category, int group, int sortIndex, int originalIndex, String source) {}

  private record StaticRewrite(List<Edit> edits, Set<String> ownerImports) {}

  private record ImportChoices(
      Map<String, String> selectedBySimpleName, Set<String> preferredImports) {
    boolean selected(String qualifiedName) {
      int separator = qualifiedName.lastIndexOf('.');
      String simpleName = separator < 0 ? qualifiedName : qualifiedName.substring(separator + 1);
      String selected = selectedBySimpleName.get(simpleName);
      return selected == null || selected.equals(qualifiedName);
    }

    boolean preferred(String qualifiedName) {
      return preferredImports.contains(qualifiedName);
    }
  }

  private record ModuleDirectiveSource(
      int category, String key, int originalIndex, String source) {}

  private record TokenSpan(int start, int end) {}

  private record Lexeme(
      String text, int start, int end, boolean comment, String kind) {}
}
