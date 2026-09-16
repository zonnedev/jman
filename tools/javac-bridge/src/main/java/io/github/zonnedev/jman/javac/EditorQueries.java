package io.github.zonnedev.jman.javac;

import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.Scope;
import com.sun.source.util.JavacTask;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.ExecutableType;
import javax.lang.model.type.ArrayType;
import javax.lang.model.type.TypeMirror;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;

final class EditorQueries {
  private static final String CURSOR = "__jman_java_cursor__";

  private EditorQueries() {}

  static EditorQueryResult query(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int cursor,
      int release) {
    return query(compiler, files, fileName, source, cursor, release, List.of());
  }

  static EditorQueryResult query(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int cursor,
      int release,
      List<String> compilerOptions) {
    int safeCursor = Math.max(0, Math.min(cursor, source.length()));
    HoverResult hoverResult =
        hover(compiler, files, fileName, source, safeCursor, release, compilerOptions);
    if (hoverResult == null || hoverResult.definition() == null) {
      HoverResult focused =
          lexicalStaticMemberHover(
              compiler, files, fileName, source, safeCursor, release, compilerOptions);
      if (focused != null) hoverResult = focused;
    }
    EditorHover hover = hoverResult == null ? null : hoverResult.hover();
    EditorDefinition definition = hoverResult == null ? null : hoverResult.definition();
    EditorDefinition typeDefinition = hoverResult == null ? null : hoverResult.typeDefinition();
    int prefixStart = safeCursor;
    while (prefixStart > 0
        && Character.isJavaIdentifierPart(source.charAt(prefixStart - 1))) {
      prefixStart--;
    }
    int dot = prefixStart - 1;
    if (dot < 0 || source.charAt(dot) != '.') {
      return new EditorQueryResult(
          List.of(),
          signaturesFromCall(
              compiler, files, fileName, source, safeCursor, release, compilerOptions),
          hover,
          definition,
          typeDefinition);
    }
    String prefix = source.substring(prefixStart, safeCursor);
    int identifierEnd = safeCursor;
    while (identifierEnd < source.length()
        && Character.isJavaIdentifierPart(source.charAt(identifierEnd))) {
      identifierEnd++;
    }
    String invocation =
        identifierEnd < source.length() && source.charAt(identifierEnd) == '(' ? "" : "()";
    String synthetic =
        source.substring(0, prefixStart) + CURSOR + invocation + source.substring(identifierEnd);
    TypeMirror[] receiver = new TypeMirror[1];
    Scope[] scope = new Scope[1];
    QueryTask originalQuery = task(compiler, files, fileName, source, release, compilerOptions);
    if (originalQuery != null) {
      var positions = originalQuery.trees().getSourcePositions();
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitMemberSelect(MemberSelectTree node, Void unused) {
          long expressionEnd =
              positions.getEndPosition(originalQuery.unit(), node.getExpression());
          long memberEnd = positions.getEndPosition(originalQuery.unit(), node);
          if (expressionEnd < safeCursor && safeCursor <= memberEnd) {
            TreePath expression = new TreePath(getCurrentPath(), node.getExpression());
            receiver[0] = originalQuery.trees().getTypeMirror(expression);
            scope[0] = originalQuery.trees().getScope(getCurrentPath());
          }
          return super.visitMemberSelect(node, unused);
        }
      }.scan(originalQuery.unit(), null);
    }
    QueryTask query = originalQuery;
    if (!(receiver[0] instanceof DeclaredType)) {
      QueryTask syntheticQuery =
          task(compiler, files, fileName, synthetic, release, compilerOptions);
      if (syntheticQuery == null) {
        return new EditorQueryResult(List.of(), List.of(), hover, definition, typeDefinition);
      }
      receiver[0] = null;
      scope[0] = null;
      new TreePathScanner<Void, Void>() {
        @Override
        public Void visitMemberSelect(MemberSelectTree node, Void unused) {
          if (node.getIdentifier().contentEquals(CURSOR)) {
            TreePath expression = new TreePath(getCurrentPath(), node.getExpression());
            receiver[0] = syntheticQuery.trees().getTypeMirror(expression);
            scope[0] = syntheticQuery.trees().getScope(getCurrentPath());
          }
          return super.visitMemberSelect(node, unused);
        }
      }.scan(syntheticQuery.unit(), null);
      query = syntheticQuery;
    }
    if (!(receiver[0] instanceof DeclaredType declared)) {
      return new EditorQueryResult(List.of(), List.of(), hover, definition, typeDefinition);
    }
    TypeElement owner = (TypeElement) declared.asElement();
    Map<String, EditorCompletion> completions = new LinkedHashMap<>();
    List<EditorSignature> signatures = new ArrayList<>();
    for (Element member : query.task().getElements().getAllMembers(owner)) {
      if ((scope[0] != null && !query.trees().isAccessible(scope[0], member, declared))
          || !member.getSimpleName().toString().startsWith(prefix)) {
        continue;
      }
      String name = member.getSimpleName().toString();
      String kind = member.getKind().name().toLowerCase(Locale.ROOT);
      TypeMirror memberType = query.task().getTypes().asMemberOf(declared, member);
      String detail = memberType.toString();
      String insert = name;
      if (member instanceof ExecutableElement executable) {
        String documentation = documentation(compiler, files, query.task(), member, release);
        EditorSignature signature =
            signature(executable, (ExecutableType) memberType, documentation);
        signatures.add(signature);
        detail = signature.label();
        insert = name + "(";
        completions.putIfAbsent(kind + ":" + name + ":" + detail,
            new EditorCompletion(name, kind, detail, insert, documentation));
        continue;
      }
      completions.putIfAbsent(kind + ":" + name + ":" + detail,
          new EditorCompletion(
              name, kind, detail, insert, documentation(compiler, files, query.task(), member, release)));
    }
    List<EditorCompletion> ordered = new ArrayList<>(completions.values());
    ordered.sort(Comparator.comparing(EditorCompletion::label)
        .thenComparing(EditorCompletion::detail));
    signatures.sort(Comparator.comparing(EditorSignature::label));
    return new EditorQueryResult(ordered, signatures, hover, definition, typeDefinition);
  }

  private static HoverResult hover(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int cursor,
      int release,
      List<String> compilerOptions) {
    QueryTask query = task(compiler, files, fileName, source, release, compilerOptions);
    if (query == null) return null;
    Element[] best = new Element[1];
    long[] bestLength = {Long.MAX_VALUE};
    var positions = query.trees().getSourcePositions();
    new TreePathScanner<Void, Void>() {
      @Override
      public Void scan(com.sun.source.tree.Tree tree, Void unused) {
        if (tree != null) {
          TreePath path = TreePath.getPath(query.unit(), tree);
          if (path == null) return super.scan(tree, unused);
          long start = positions.getStartPosition(query.unit(), tree);
          long end = positions.getEndPosition(query.unit(), tree);
          if (start >= 0 && end < start) end = start + tree.toString().length();
          if (start <= cursor && cursor <= end && end - start < bestLength[0]) {
            Element element = query.trees().getElement(path);
            if (element != null) {
              best[0] = element;
              bestLength[0] = end - start;
            }
          }
        }
        return super.scan(tree, unused);
      }
    }.scan(query.unit(), null);
    if (best[0] == null) return null;
    if (SymbolIds.isUnresolvedRecovery(query.task().getElements(), best[0])) {
      String packageName =
          query.unit().getPackageName() == null ? "" : query.unit().getPackageName().toString();
      String candidate =
          packageName.isEmpty()
              ? best[0].getSimpleName().toString()
              : packageName + "." + best[0].getSimpleName();
      Element recovered = query.task().getElements().getTypeElement(candidate);
      if (recovered == null
          || SymbolIds.isUnresolvedRecovery(query.task().getElements(), recovered)) return null;
      best[0] = recovered;
    }
    String documentation = documentation(compiler, files, query.task(), best[0], release);
    EditorDefinition definition =
        JavadocSourceIndex.definition(compiler, files, query.task(), best[0], release);
    if (definition == null) {
      definition = binaryDefinition(query.task(), best[0]);
    }
    EditorDefinition typeDefinition = typeDefinition(compiler, files, query, best[0], release);
    return new HoverResult(
        new EditorHover(best[0].asType().toString(), documentation), definition, typeDefinition);
  }

  private static EditorDefinition typeDefinition(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      QueryTask query,
      Element element,
      int release) {
    TypeMirror type = element instanceof ExecutableElement executable
        ? executable.getReturnType()
        : element.asType();
    while (type instanceof ArrayType array) type = array.getComponentType();
    Element target = query.task().getTypes().asElement(type);
    if (!(target instanceof TypeElement)) return null;
    EditorDefinition definition =
        JavadocSourceIndex.definition(compiler, files, query.task(), target, release);
    return definition == null ? binaryDefinition(query.task(), target) : definition;
  }

  private static HoverResult lexicalStaticMemberHover(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int cursor,
      int release,
      List<String> compilerOptions) {
    int memberStart = cursor;
    while (memberStart > 0
        && Character.isJavaIdentifierPart(source.charAt(memberStart - 1))) {
      memberStart--;
    }
    int memberEnd = cursor;
    while (memberEnd < source.length()
        && Character.isJavaIdentifierPart(source.charAt(memberEnd))) {
      memberEnd++;
    }
    if (memberStart == memberEnd || memberStart == 0 || source.charAt(memberStart - 1) != '.') {
      return null;
    }
    int ownerEnd = memberStart - 1;
    int ownerStart = ownerEnd;
    while (ownerStart > 0
        && Character.isJavaIdentifierPart(source.charAt(ownerStart - 1))) {
      ownerStart--;
    }
    if (ownerStart == ownerEnd) return null;
    String ownerName = source.substring(ownerStart, ownerEnd);
    String memberName = source.substring(memberStart, memberEnd);
    java.util.regex.Matcher imported =
        java.util.regex.Pattern.compile(
                "\\bimport\\s+([\\w.]+\\." + java.util.regex.Pattern.quote(ownerName) + ")\\s*;")
            .matcher(source);
    String qualifiedOwner =
        imported.find()
            ? imported.group(1)
            : switch (ownerName) {
              case "String", "Object", "Class", "System", "Math", "Thread", "Throwable" ->
                  "java.lang." + ownerName;
              default -> null;
            };
    if (qualifiedOwner == null) return null;
    StringBuilder preamble = new StringBuilder();
    java.util.regex.Matcher declarations =
        java.util.regex.Pattern.compile("(?m)^\\s*(?:package|import)\\s+[^;]+;")
            .matcher(source);
    while (declarations.find()) preamble.append(declarations.group()).append('\n');
    String focused = preamble + "final class __JavaLspFocusedQuery {}";
    QueryTask query = task(compiler, files, fileName, focused, release, compilerOptions);
    if (query == null) return null;
    TypeElement owner = query.task().getElements().getTypeElement(qualifiedOwner);
    if (owner == null) return null;
    Element member =
        query.task().getElements().getAllMembers(owner).stream()
            .filter(candidate -> candidate.getSimpleName().contentEquals(memberName))
            .findFirst()
            .orElse(null);
    if (member == null) return null;
    String documentation = documentation(compiler, files, query.task(), member, release);
    EditorDefinition definition = JavadocSourceIndex.definition(
        compiler, files, query.task(), member, release);
    if (definition == null) definition = binaryDefinition(query.task(), member);
    return new HoverResult(
        new EditorHover(member.asType().toString(), documentation),
        definition,
        typeDefinition(compiler, files, query, member, release));
  }

  private static EditorDefinition binaryDefinition(JavacTask task, Element element) {
    Element current = element instanceof TypeElement ? element : element.getEnclosingElement();
    while (current != null && !(current instanceof TypeElement)) {
      current = current.getEnclosingElement();
    }
    if (!(current instanceof TypeElement owner)) return null;
    return new EditorDefinition(
        SymbolIds.of(task.getElements(), task.getTypes(), element),
        SymbolIds.moduleName(task.getElements(), element),
        SymbolIds.binaryOwner(task.getElements(), element),
        element.getSimpleName().toString(),
        SymbolIds.descriptor(task.getElements(), task.getTypes(), element),
        "",
        "",
        0,
        0,
        true);
  }

  private static List<EditorSignature> signaturesFromCall(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int cursor,
      int release,
      List<String> compilerOptions) {
    int open = source.lastIndexOf('(', Math.max(0, cursor - 1));
    if (open < 0) return List.of();
    int nameStart = open;
    while (nameStart > 0
        && Character.isJavaIdentifierPart(source.charAt(nameStart - 1))) nameStart--;
    if (nameStart == open) return List.of();
    String name = source.substring(nameStart, open);
    int dot = nameStart - 1;
    if (dot < 0 || source.charAt(dot) != '.') return List.of();
    EditorQueryResult members =
        query(compiler, files, fileName, source, open, release, compilerOptions);
    return members.signatures().stream()
        .filter(signature -> signature.label().contains(" " + name + "("))
        .toList();
  }

  private static EditorSignature signature(
      ExecutableElement method, ExecutableType memberType, String documentation) {
    List<String> parameters = new ArrayList<>();
    for (int index = 0; index < method.getParameters().size(); index++) {
      parameters.add(
          memberType.getParameterTypes().get(index)
              + " "
              + method.getParameters().get(index).getSimpleName());
    }
    String returnType = method.getKind() == ElementKind.CONSTRUCTOR
        ? ""
        : memberType.getReturnType().toString();
    String label = (returnType.isEmpty() ? "" : returnType + " ")
        + method.getSimpleName() + "(" + String.join(", ", parameters) + ")";
    return new EditorSignature(label, parameters, returnType, documentation);
  }

  private static String documentation(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      JavacTask task,
      Element element,
      int release) {
    String comment = task.getElements().getDocComment(element);
    if (comment != null && !comment.isBlank()) return JavadocMarkdown.render(comment);
    return JavadocSourceIndex.lookup(compiler, files, element, release);
  }

  private static QueryTask task(
      JavaCompiler compiler,
      StandardJavaFileManager files,
      String fileName,
      String source,
      int release,
      List<String> compilerOptions) {
    DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
    List<String> options =
        new ArrayList<>(List.of("-proc:none", "--release", Integer.toString(release)));
    options.addAll(compilerOptions);
    JavacTask task = (JavacTask) compiler.getTask(
        null, files, diagnostics,
        options,
        null, List.of(new StringJavaFileObject(fileName, source)));
    CompilationUnitTree unit;
    try {
      unit = task.parse().iterator().next();
    } catch (IOException | RuntimeException | AssertionError failure) {
      return null;
    }
    try {
      task.analyze();
    } catch (IOException | RuntimeException | AssertionError failure) {
      // Editor buffers routinely contain incomplete code, and javac can still
      // provide useful attributed trees before reporting an internal
      // completion failure. Keep that partial task for hover/definition.
    }
    return new QueryTask(task, Trees.instance(task), unit);
  }

  private record QueryTask(JavacTask task, Trees trees, CompilationUnitTree unit) {}

  private record HoverResult(
      EditorHover hover, EditorDefinition definition, EditorDefinition typeDefinition) {}
}
