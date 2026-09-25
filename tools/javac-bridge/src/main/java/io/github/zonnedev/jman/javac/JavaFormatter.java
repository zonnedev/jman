package io.github.zonnedev.jman.javac;

import com.sun.source.tree.ClassTree;
import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.ImportTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.Tree;
import com.sun.source.tree.VariableTree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePathScanner;
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
import javax.lang.model.element.NestingKind;
import javax.lang.model.element.TypeElement;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;

/** The canonical JMAN Java source formatter. */
final class JavaFormatter {
  private static final String INDENT = "    ";
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
      List<Edit> edits = structuralEdits(source, unit, positions, task);

      boolean attributionComplete = true;
      try {
        task.analyze();
      } catch (RuntimeException ignored) {
        // Attribution is best effort. Syntax-only formatting remains available.
        attributionComplete = false;
      }
      attributionComplete &= convertErrors(diagnostics).isEmpty();

      Edit imports =
          expandedImports(source, unit, trees, task, positions, attributionComplete);
      if (imports != null) edits.add(imports);
      String normalized = applyEdits(source, edits);
      String formatted = formatTokens(normalized, task);
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

  private static List<Edit> structuralEdits(
      String source, CompilationUnitTree unit, SourcePositions positions, JavacTask task) {
    List<Edit> edits = new ArrayList<>();
    for (Tree declaration : unit.getTypeDecls()) {
      if (declaration instanceof ClassTree type) {
        Edit members = orderedMembers(source, unit, type, positions, task);
        if (members != null) edits.add(members);
      }
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

  private static String applyEdits(String source, List<Edit> edits) {
    edits.sort(Comparator.comparingInt(Edit::start).reversed());
    StringBuilder normalized = new StringBuilder(source);
    for (Edit edit : edits) normalized.replace(edit.start(), edit.end(), edit.replacement());
    return normalized.toString();
  }

  private static Edit expandedImports(
      String source,
      CompilationUnitTree unit,
      Trees trees,
      JavacTask task,
      SourcePositions positions,
      boolean attributionComplete) {
    if (unit.getImports().isEmpty()) return null;
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
        Set<String> expansion =
            wildcardStatics.getOrDefault(name.substring(0, name.length() - 2), Set.of());
        statics.addAll(expansion);
      } else if (imported.isStatic()) {
        statics.add(name);
      } else {
        ordinary.add(name);
      }
    }
    List<String> lines = new ArrayList<>();
    ordinary.stream().sorted().map(name -> "import " + name + ";").forEach(lines::add);
    if (!ordinary.isEmpty() && !statics.isEmpty()) lines.add("");
    statics.stream().sorted().map(name -> "import static " + name + ";").forEach(lines::add);

    int start = Math.toIntExact(positions.getStartPosition(unit, unit.getImports().get(0)));
    int end =
        Math.toIntExact(
            positions.getEndPosition(unit, unit.getImports().get(unit.getImports().size() - 1)));
    String leading = unit.getPackage() == null ? "" : "\n\n";
    return new Edit(
        start, end, lines.isEmpty() ? "" : leading + String.join("\n", lines) + "\n\n");
  }

  private static void addType(Element element, Set<TypeElement> referenced) {
    if (element instanceof TypeElement type) referenced.add(type);
  }

  private static Edit orderedMembers(
      String source,
      CompilationUnitTree unit,
      ClassTree type,
      SourcePositions positions,
      JavacTask task) {
    List<MemberSource> members = new ArrayList<>();
    int declarationStart = Math.toIntExact(positions.getStartPosition(unit, type));
    int declarationEnd = Math.toIntExact(positions.getEndPosition(unit, type));
    if (declarationStart < 0 || declarationEnd <= declarationStart) return null;

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
        Edit nestedEdit = orderedMembers(source, unit, nested, positions, task);
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
      chunks.add(
          new MemberChunk(
              memberCategory(member.tree()), memberName(member.tree()), member.index(), chunk));
      cursor = chunkEnd;
    }
    chunks.sort(
        Comparator.comparingInt(MemberChunk::category)
            .thenComparing(MemberChunk::name)
            .thenComparingInt(MemberChunk::originalIndex));
    String ordered = String.join("\n\n", chunks.stream().map(MemberChunk::source).toList());
    String replacement = prefix.isEmpty() ? "\n" + ordered + "\n" : prefix + "\n" + ordered + "\n";
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

  private static int memberCategory(Tree tree) {
    if (tree instanceof VariableTree) return 10;
    if (tree.getKind() == Tree.Kind.BLOCK) return 10;
    if (tree instanceof MethodTree method) return method.getReturnType() == null ? 30 : 40;
    if (tree instanceof ClassTree) return 50;
    return 60;
  }

  private static String memberName(Tree tree) {
    // Field and initializer order is observable at runtime, so preserve it.
    if (tree instanceof VariableTree || tree.getKind() == Tree.Kind.BLOCK) return "";
    if (tree instanceof MethodTree method) return method.getName().toString();
    if (tree instanceof ClassTree nested) return nested.getSimpleName().toString();
    return "";
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

    Renderer(String source, List<Lexeme> lexemes) {
      this.source = source;
      this.lexemes = lexemes;
    }

    String render() {
      Lexeme previous = null;
      for (int index = 0; index < lexemes.size(); index++) {
        Lexeme current = lexemes.get(index);
        String gap = source.substring(Math.min(previousEnd, current.start()), current.start());
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

    private void comment(Lexeme comment, String gap) {
      boolean lineComment = comment.kind().equals("LINE") || comment.kind().equals("JAVADOC_LINE");
      if (!lineStart) {
        if (gap.contains("\n")) newline(blankGap(gap));
        else space();
      }
      append(comment.text());
      if (lineComment || gap.contains("\n")) newline(false);
    }

    private void token(int index, Lexeme previous, Lexeme current, String gap) {
      String text = current.text();
      String before = previous == null ? "" : previous.text();
      boolean closingBrace = text.equals("}");
      if (closingBrace) {
        indent = Math.max(0, indent - 1);
        if (!before.equals("{") && !lineStart) newline(false);
      } else if (previous == null && !lineStart && gap.contains("\n")) {
        newline(blankGap(gap));
      } else if (previous != null) {
        if (before.equals("{") || (before.equals(";") && parenthesisDepth == 0)) {
          newline(blankGap(gap));
        } else if (before.equals("}") && !Set.of("else", "catch", "finally", "while").contains(text)
            && !Set.of(";", ",", ")").contains(text)) {
          newline(blankGap(gap));
        } else if (blankGap(gap) && lineStart) {
          newline(true);
        } else if (needsSpace(index, before, text)) {
          space();
        }
      }

      append(text);
      if (text.equals("{")) indent++;
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

    private boolean needsSpace(int index, String previous, String current) {
      if (Set.of(")", "]", ",", ";", ".", "::").contains(current)) return false;
      if (Set.of("(", "[", ".", "::", "@").contains(previous)) return false;
      if (current.equals("(")) {
        if (previousGenericClose) return false;
        return CONTROL_PARENTHESIS.contains(previous)
            || Set.of("return", "throw", "assert", "yield").contains(previous)
            || BINARY_OPERATORS.contains(previous);
      }
      if (current.equals("{") || Set.of("else", "catch", "finally").contains(current)) return true;
      if (previous.equals(",")) return true;
      if (previous.equals("]") && word(current)) return true;
      if (previous.equals("...")) return true;
      if (current.equals("?") && genericDepth == 0) return true;
      if (previous.equals("?") && genericDepth == 0) return true;
      if (current.equals(":")) return ternaryDepth > 0 || parenthesisDepth > 0;
      if (previous.equals(":")) return true;
      if (previous.equals("?") && Set.of("extends", "super").contains(current)) return true;
      if (current.equals("<") && genericOpening(index)) return false;
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

  private record MemberChunk(int category, String name, int originalIndex, String source) {}

  private record Lexeme(
      String text, int start, int end, boolean comment, String kind) {}
}
