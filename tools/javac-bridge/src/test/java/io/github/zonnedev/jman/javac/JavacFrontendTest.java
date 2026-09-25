package io.github.zonnedev.jman.javac;

final class JavacFrontendTest {
  public static void main(String[] args) {
    parsesTopLevelAndNestedDeclarations();
    reportsMalformedSourceWithoutThrowing();
    recognizesJava25Declarations();
    parsesWorkspaceBatchWithoutAttribution();
    parsesSupportedReleaseAndPreviewSyntax();
    resolvesReceiverMembersAndOverloadSignatures();
    resolvesInheritanceGenericsAccessOverloadsAndMethodReferences();
    rendersProjectJavadocAsMarkdown();
    readsJdkJavadocWhenSourceArchiveIsAvailable();
    attributesJdkAndSourceSymbols();
    emitsCanonicalJvmSymbolIds();
    classifiesVariableWritesAndReads();
    emitsOverrideFamiliesForDeclarationsCallsAndMethodReferences();
    emitsPackageDeclarationsForSafePackageRename();
    doesNotResolveTypesWithMissingImports();
    enforcesJpmsReadabilityExportsAndModuleIdentity();
    supportsTransitiveServicesQualifiedAccessAndCompilerOverrides();
    selectsMultiReleaseJarApisByProjectRelease();
    sessionUsesTheLatestUnsavedBuffer();
    selectsProcessingModeFromCallerOptions();
    formatsJavaFromJavacTokensAndPreservesComments();
    expandsWildcardImportsAndOrdersMembers();
    expandsStaticWildcardsAndPreservesHeaderCommentsAndInitializerOrder();
    formatsModernJavaSyntaxIdempotently();
    selectsFormattingReleaseAndOnlyRelevantCompilerOptions();
    reordersDocumentedInterfaceMethodsWithoutDetachingJavadocs();
    ordersMembersWrittenOnOneLine();
    formatsAFileThatIsAlsoPresentOnTheProjectSourcePath();
    preservesWildcardImportsWhenAttributionIsIncomplete();
    wireFormatIsVersionedAndDeterministic();
  }

  private static void formatsJavaFromJavacTokensAndPreservesComments() {
    String source =
        "class Messy{ // type comment\n"
            + "void run( ){if  (value== 1) {call( 1,2 ); /* keep me */}}\n"
            + "}\n";
    FormatResult result =
        JavaFormatter.format(
            "Messy.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(result.diagnostics().isEmpty(), result.diagnostics().toString());
    assertEquals(
        "class Messy { // type comment\n"
            + "    void run() {\n"
            + "        if (value == 1) {\n"
            + "            call(1, 2); /* keep me */\n"
            + "        }\n"
            + "    }\n"
            + "}\n",
        result.source());
    FormatResult second =
        JavaFormatter.format(
            "Messy.java", result.source(), java.util.List.of(), java.util.List.of(), 25);
    assertEquals(result.source(), second.source());
  }

  private static void expandsWildcardImportsAndOrdersMembers() {
    String source =
        "import java.util.*;\n"
            + "class Ordered {\n"
            + "    // zebra docs\n"
            + "    void zebra() {} // zebra trailing\n"
            + "    private int value;\n"
            + "    void alpha() { List<String> values = new ArrayList<>(); }\n"
            + "}\n";
    FormatResult result =
        JavaFormatter.format(
            "Ordered.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(result.diagnostics().isEmpty(), result.diagnostics().toString());
    assertEquals(
        "import java.util.ArrayList;\n"
            + "import java.util.List;\n"
            + "\n"
            + "class Ordered {\n"
            + "    private int value;\n"
            + "\n"
            + "    void alpha() {\n"
            + "        List<String> values = new ArrayList<>();\n"
            + "    }\n"
            + "\n"
            + "    // zebra docs\n"
            + "    void zebra() {} // zebra trailing\n"
            + "}\n",
        result.source());
  }

  private static void expandsStaticWildcardsAndPreservesHeaderCommentsAndInitializerOrder() {
    String source =
        "package demo; import java.util.*; import static java.util.Collections.*;\n"
            + "class Ordered { // header\n"
            + "  static int zeta=1; static { zeta++; } static int alpha=zeta+1;\n"
            + "  void zebra(){sort(new ArrayList<String>());}\n"
            + "  void alpha(String[]args){}\n"
            + "}\n";
    FormatResult result =
        JavaFormatter.format(
            "Ordered.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(result.diagnostics().isEmpty(), result.diagnostics().toString());
    assertEquals(
        "package demo;\n"
            + "\n"
            + "import java.util.ArrayList;\n"
            + "\n"
            + "import static java.util.Collections.sort;\n"
            + "\n"
            + "class Ordered { // header\n"
            + "    static int zeta = 1;\n"
            + "\n"
            + "    static {\n"
            + "        zeta++;\n"
            + "    }\n"
            + "\n"
            + "    static int alpha = zeta + 1;\n"
            + "\n"
            + "    void alpha(String[] args) {}\n"
            + "\n"
            + "    void zebra() {\n"
            + "        sort(new ArrayList<String>());\n"
            + "    }\n"
            + "}\n",
        result.source());
  }

  private static void formatsModernJavaSyntaxIdempotently() {
    String source =
        "import java.util.function.*;\n"
            + "@Deprecated record Modern<T>(T value){\n"
            + "  static final String TEXT=\"\"\"\nhello\n\"\"\";\n"
            + "  int choose(boolean flag){int negative=-1;return flag?negative:-2;}\n"
            + "  void each(String...args){for(String arg:args){Supplier<String> task=()->arg;task.get();this.<String>consume(arg);}}\n"
            + "  <X> void consume(X value){}\n"
            + "  <R extends Comparable<? super R>> R identity(R input){return input;}\n"
            + "}\n";
    FormatResult first =
        JavaFormatter.format(
            "Modern.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(first.diagnostics().isEmpty(), first.diagnostics().toString());
    assertTrue(first.source().contains("int negative = -1;"), first.source());
    assertTrue(first.source().contains("return flag ? negative : -2;"), first.source());
    assertTrue(first.source().contains("void each(String... args)"), first.source());
    assertTrue(first.source().contains("Comparable<? super R>"), first.source());
    assertTrue(first.source().contains("this.<String>consume(arg);"), first.source());
    assertTrue(first.source().contains("import java.util.function.Supplier;"), first.source());
    assertTrue(first.source().contains("\"\"\"\nhello\n\"\"\""), first.source());
    FormatResult second =
        JavaFormatter.format(
            "Modern.java", first.source(), java.util.List.of(), java.util.List.of(), 25);
    assertEquals(first.source(), second.source());
  }

  private static void selectsFormattingReleaseAndOnlyRelevantCompilerOptions() {
    java.util.List<String> options =
        JavaFormatter.formatOptions(17, java.util.List.of("--enable-preview", "-Xlint:all"));
    assertTrue(options.contains("--release"), "formatter must select the Java release");
    assertTrue(options.contains("17"), "formatter lost the requested Java release");
    assertTrue(
        options.contains("--enable-preview"), "formatter must retain preview syntax support");
    assertTrue(!options.contains("-Xlint:all"), "formatter leaked unrelated compiler options");
  }

  private static void reordersDocumentedInterfaceMethodsWithoutDetachingJavadocs() {
    String source =
        "/* license */\ninterface OwnerRepository {\n"
            + "  /** Find {@link Owner}s.\n"
            + "   * @return matching {@link Owner}s (or an empty collection if none\n"
            + "   * found)\n"
            + "   */\n"
            + "  Page<Owner> findByLastNameStartingWith(String name, Pageable pageable);\n"
            + "  /** Find one {@link Owner}.\n   * @param id owner id\n   */\n"
            + "  Optional<Owner> findById(Integer id);\n"
            + "}\n";
    FormatResult result =
        JavaFormatter.format(
            "OwnerRepository.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(result.diagnostics().isEmpty(), result.source() + "\n" + result.diagnostics());
    assertTrue(result.source().startsWith("/* license */\ninterface"), result.source());
    assertTrue(
        result.source().indexOf("findById") < result.source().indexOf("findByLastNameStartingWith"),
        result.source());
  }

  private static void ordersMembersWrittenOnOneLine() {
    String source =
        "class Inline { class Nested { void zebra() {} void alpha() {} } int value; void zebra() {} void alpha() {} }\n";
    FormatResult result =
        JavaFormatter.format(
            "Inline.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(
        result.source().indexOf("void alpha()") < result.source().indexOf("void zebra()"),
        result.source());
    int nestedStart = result.source().indexOf("class Nested");
    int nestedEnd = result.source().indexOf("\n    }", nestedStart);
    String nested = result.source().substring(nestedStart, nestedEnd);
    assertTrue(
        nested.indexOf("void alpha()") < nested.indexOf("void zebra()"), result.source());
  }

  private static void formatsAFileThatIsAlsoPresentOnTheProjectSourcePath() {
    try {
      java.nio.file.Path root = java.nio.file.Files.createTempDirectory("jman-formatter-source-path-");
      try {
        java.nio.file.Path file = root.resolve("demo/Inline.java");
        java.nio.file.Files.createDirectories(file.getParent());
        String source =
            "package demo; import java.util.*; class Inline { List<String> values=new ArrayList<>(); void zebra() {} void alpha() {} }\n";
        java.nio.file.Files.writeString(file, source);
        FormatResult result =
            JavaFormatter.format(
                "Inline.java", source, java.util.List.of(), java.util.List.of(root), 25);
        assertTrue(
            result.source().indexOf("void alpha()") < result.source().indexOf("void zebra()"),
            result.source());
      } finally {
        try (var paths = java.nio.file.Files.walk(root)) {
          paths.sorted(java.util.Comparator.reverseOrder())
              .forEach(
                  path -> {
                    try {
                      java.nio.file.Files.deleteIfExists(path);
                    } catch (java.io.IOException exception) {
                      throw new java.io.UncheckedIOException(exception);
                    }
                  });
        }
      }
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void preservesWildcardImportsWhenAttributionIsIncomplete() {
    String source =
        "import java.util.*; class Incomplete { List<String> values; Missing unresolved; }\n";
    FormatResult result =
        JavaFormatter.format(
            "Incomplete.java", source, java.util.List.of(), java.util.List.of(), 25);
    assertTrue(result.diagnostics().isEmpty(), result.diagnostics().toString());
    assertTrue(result.source().contains("import java.util.*;"), result.source());
  }

  private static void selectsProcessingModeFromCallerOptions() {
    java.util.List<String> nativeOptions = JavacFrontend.analysisOptions(21, java.util.List.of());
    assertTrue(nativeOptions.contains("-proc:none"), "native analysis must disable processors");
    java.util.List<String> processedOptions =
        JavacFrontend.analysisOptions(21, java.util.List.of("-proc:full", "-Amode=strict"));
    assertTrue(
        processedOptions.contains("-proc:full"), "processed analysis lost its processing mode");
    assertTrue(
        !processedOptions.contains("-proc:none"),
        "processed analysis was accidentally overridden with -proc:none");
    assertTrue(processedOptions.contains("-Amode=strict"), "processor options were not preserved");
  }

  private static void readsJdkJavadocWhenSourceArchiveIsAvailable() {
    java.nio.file.Path sourceArchive =
        java.nio.file.Path.of(System.getProperty("java.home"), "lib", "src.zip");
    if (!java.nio.file.Files.isRegularFile(sourceArchive)) return;
    javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
    try (javax.tools.StandardJavaFileManager files =
        compiler.getStandardFileManager(null, java.util.Locale.ROOT, null)) {
      files.setLocationFromPaths(
          javax.tools.StandardLocation.SOURCE_PATH, java.util.List.of(sourceArchive));
      String source =
          "import java.util.*; class Demo { void test() { List<String> values = null; values.ad; } }";
      int cursor = source.indexOf("values.ad") + "values.ad".length();
      EditorQueryResult query =
          EditorQueries.query(compiler, files, "Demo.java", source, cursor, 25);
      EditorCompletion add =
          query.completions().stream()
              .filter(completion -> completion.label().equals("add"))
              .findFirst()
              .orElseThrow();
      assertTrue(!add.documentation().isBlank(), "JDK List.add Javadoc was not loaded");
      String invocation =
          "import java.util.*; class Demo { void test() { List<String> values=null; values.add(\"x\"); } }";
      int invocationCursor = invocation.indexOf(".add") + 2;
      EditorDefinition definition =
          EditorQueries.query(
                  compiler, files, "Demo.java", invocation, invocationCursor, 25)
              .definition();
      assertTrue(definition != null, "JDK List.add definition was not located");
      assertEquals("java.base", definition.module());
      assertEquals("(Ljava/lang/Object;)Z", definition.descriptor());
      assertEquals(
          "java.base|java/util/List#add(Ljava/lang/Object;)Z", definition.symbolId());
      assertTrue(definition.source().contains("interface List"), definition.sourceName());
      assertEquals(
          "add",
          definition.source().substring(
              Math.toIntExact(definition.start()), Math.toIntExact(definition.end())));
      String overloadedInvocation =
          "import java.util.*; class Demo { void test() { List<String> values=null; values.remove(0); } }";
      int overloadedCursor = overloadedInvocation.indexOf(".remove") + 2;
      EditorDefinition overload =
          EditorQueries.query(
                  compiler, files, "Demo.java", overloadedInvocation, overloadedCursor, 25)
              .definition();
      assertTrue(overload != null, "JDK List.remove(int) definition was not located");
      int lineStart = overload.source().lastIndexOf('\n', Math.toIntExact(overload.start())) + 1;
      int lineEnd = overload.source().indexOf('\n', Math.toIntExact(overload.end()));
      assertTrue(
          overload.source().substring(lineStart, lineEnd).contains("remove(int index)"),
          overload.source().substring(lineStart, lineEnd));
      String constructorInvocation = "class Demo { Demo() { super(); } }";
      int constructorCursor = constructorInvocation.indexOf("super") + 2;
      EditorDefinition constructor =
          EditorQueries.query(
                  compiler,
                  files,
                  "Demo.java",
                  constructorInvocation,
                  constructorCursor,
                  25)
              .definition();
      assertTrue(constructor != null, "Object() constructor definition was not located");
      assertTrue(constructor.sourceName().endsWith("Object.java"), constructor.sourceName());
      assertEquals(
          "Object",
          constructor.source().substring(
              Math.toIntExact(constructor.start()), Math.toIntExact(constructor.end())));
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void emitsCanonicalJvmSymbolIds() {
    String source =
        "package demo; class Outer { static class Box<T> { void set(T[] value) {} void set(String value) {} Box() {} } }";
    SemanticResult result =
        JavacFrontend.analyze(
            "Outer.java", source, java.util.List.of(), java.util.List.of(), 25);
    java.util.Set<String> methods =
        result.symbols().stream()
            .filter(
                symbol ->
                    symbol.role().equals("declaration")
                        && (symbol.name().equals("set") || symbol.name().equals("<init>")))
            .map(SemanticSymbol::symbolId)
            .collect(java.util.stream.Collectors.toSet());
    assertTrue(
        methods.contains("<unnamed>|demo/Outer$Box#set([Ljava/lang/Object;)V"), methods.toString());
    assertTrue(
        methods.contains("<unnamed>|demo/Outer$Box#set(Ljava/lang/String;)V"), methods.toString());
    assertTrue(methods.contains("<unnamed>|demo/Outer$Box#<init>()V"), methods.toString());
  }

  private static void classifiesVariableWritesAndReads() {
    String source =
        "class Counter { void use(int value) {} void run() { int value = 0; value = 1; value++; use(value); } }";
    SemanticResult result =
        JavacFrontend.analyze("Counter.java", source, java.util.List.of(), java.util.List.of(), 25);
    long writes =
        result.symbols().stream()
            .filter(symbol -> symbol.name().equals("value") && symbol.role().equals("write"))
            .count();
    long reads =
        result.symbols().stream()
            .filter(symbol -> symbol.name().equals("value") && symbol.role().equals("reference"))
            .count();
    assertEquals(2L, writes);
    assertTrue(reads >= 1, "variable read was not classified");
  }

  private static void emitsOverrideFamiliesForDeclarationsCallsAndMethodReferences() {
    String source =
        """
        interface Service { String value(); }
        class Parent implements Service { public String value() { return ""; } }
        class Child extends Parent {
          @Override public String value() { return super.value(); }
          java.util.function.Supplier<String> supplier = this::value;
        }
        """;
    SemanticResult result =
        JavacFrontend.analyze(
            "Child.java", source, java.util.List.of(), java.util.List.of(), 25);
    java.util.Set<String> exactIds =
        result.symbols().stream()
            .filter(
                symbol ->
                    symbol.role().equals("override_family")
                        && symbol.name().equals("value"))
            .map(SemanticSymbol::symbolId)
            .collect(java.util.stream.Collectors.toSet());
    java.util.Set<String> families =
        result.symbols().stream()
            .filter(
                symbol ->
                    symbol.role().equals("override_family")
                        && symbol.name().equals("value"))
            .map(SemanticSymbol::qualifiedName)
            .collect(java.util.stream.Collectors.toSet());
    assertTrue(exactIds.size() >= 3, exactIds.toString());
    assertEquals(1, families.size());
    assertTrue(
        families.iterator().next().contains("Service#value"),
        families.toString());
    assertTrue(
        result.symbols().stream()
            .anyMatch(
                symbol ->
                    symbol.role().equals("call_edge")
                        && symbol.name().equals("value")
                        && symbol.qualifiedName().contains("Child#value")),
        "method invocation did not retain its enclosing caller");
    assertTrue(
        result.symbols().stream()
            .anyMatch(
                symbol ->
                    symbol.role().equals("type_edge")
                        && symbol.name().equals("Parent")
                        && symbol.qualifiedName().contains("Child")),
        "direct superclass edge was not retained");
  }

  private static void emitsPackageDeclarationsForSafePackageRename() {
    String source = "package io.github.demo; public class Demo {}";
    SemanticResult result =
        JavacFrontend.analyze(
            "Demo.java", source, java.util.List.of(), java.util.List.of(), 25);
    SemanticSymbol declaration =
        result.symbols().stream()
            .filter(symbol -> symbol.role().equals("declaration") && symbol.kind().equals("package"))
            .findFirst()
            .orElseThrow();
    assertEquals("demo", declaration.name());
    assertEquals("io.github.demo", declaration.qualifiedName());
    assertEquals(
        "demo",
        source.substring(
            Math.toIntExact(declaration.start()), Math.toIntExact(declaration.end())));
  }

  private static void doesNotResolveTypesWithMissingImports() {
    javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
    try (javax.tools.StandardJavaFileManager files =
        compiler.getStandardFileManager(null, java.util.Locale.ROOT, null)) {
      String source = "class Demo { HashMap<String, String> values; }";
      int cursor = source.indexOf("HashMap") + 2;
      EditorQueryResult query =
          EditorQueries.query(compiler, files, "Demo.java", source, cursor, 25);
      assertTrue(query.definition() == null, "missing import resolved to " + query.definition());
      SemanticResult semantic =
          JavacFrontend.analyze("Demo.java", source, java.util.List.of(), java.util.List.of(), 25);
      assertTrue(
          semantic.symbols().stream()
              .noneMatch(symbol -> symbol.symbolId().contains("java/util/HashMap")),
          semantic.symbols().toString());
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void rendersProjectJavadocAsMarkdown() {
    javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
    try (javax.tools.StandardJavaFileManager files =
        compiler.getStandardFileManager(null, java.util.Locale.ROOT, null)) {
      String source =
          """
          class Demo {
            static class Helper {
              /**
               * Greets the {@code person}.
               * @param person person to greet
               * @return the greeting
               * @throws IllegalArgumentException when empty
               * @since 1.0
               */
              String greet(String person) { return person; }
            }
            void test() { Helper helper = null; helper.gr }
          }
          """;
      int cursor = source.indexOf("helper.gr") + "helper.gr".length();
      EditorQueryResult result =
          EditorQueries.query(compiler, files, "Demo.java", source, cursor, 25);
      EditorCompletion greet =
          result.completions().stream()
              .filter(completion -> completion.label().equals("greet"))
              .findFirst()
              .orElseThrow();
      assertTrue(greet.documentation().contains("Greets the `person`"), greet.documentation());
      assertTrue(greet.documentation().contains("**Parameters**"), greet.documentation());
      assertTrue(greet.documentation().contains("person to greet"), greet.documentation());
      assertTrue(greet.documentation().contains("**Returns**"), greet.documentation());
      assertTrue(greet.documentation().contains("**Throws**"), greet.documentation());
      assertTrue(greet.documentation().contains("**Since:** 1.0"), greet.documentation());
      String hoverSource = source.replace("helper.gr", "helper.greet(\"world\")");
      int hoverCursor = hoverSource.indexOf("helper.greet") + "helper.gre".length();
      EditorHover hover =
          EditorQueries.query(compiler, files, "Demo.java", hoverSource, hoverCursor, 25).hover();
      assertTrue(hover != null, "hover symbol was not resolved");
      assertTrue(hover.documentation().contains("Greets the `person`"), hover.documentation());
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void resolvesReceiverMembersAndOverloadSignatures() {
    javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
    try (javax.tools.StandardJavaFileManager files =
        compiler.getStandardFileManager(null, java.util.Locale.ROOT, null)) {
      String completionSource =
          "import java.util.*; class Demo { void test() { List<String> values = null; values.ad; } }";
      int completionCursor = completionSource.indexOf("values.ad") + "values.ad".length();
      EditorQueryResult completion =
          EditorQueries.query(
              compiler, files, "Demo.java", completionSource, completionCursor, 25);
      assertTrue(
          completion.completions().stream()
              .anyMatch(item -> item.label().equals("add") && item.detail().contains("boolean")),
          "receiver-aware completion missed List.add: " + completion.completions());

      String signatureSource =
          "import java.util.*; class Demo { void test() { List<String> values = null; values.add( } }";
      EditorQueryResult signature =
          EditorQueries.query(
              compiler, files, "Demo.java", signatureSource, signatureSource.indexOf("add(") + 4, 25);
      assertTrue(
          signature.signatures().stream()
              .anyMatch(item -> item.label().contains("add(") && !item.parameters().isEmpty()),
          "signature query missed List.add overloads: " + signature.signatures());
      assertEquals(
          "JFQ2",
          new String(
              WireEncoder.encode(completion),
              0,
              4,
              java.nio.charset.StandardCharsets.US_ASCII));
      String typeSource =
          "import java.util.*; class Demo { void test() { List<String> values = null; values.add(\"x\"); } }";
      int typeCursor = typeSource.indexOf("values.add") + 2;
      EditorQueryResult typeQuery =
          EditorQueries.query(compiler, files, "Demo.java", typeSource, typeCursor, 25);
      EditorDefinition typeDefinition = typeQuery.typeDefinition();
      assertTrue(
          typeDefinition != null,
          "List type definition was not resolved: hover=" + typeQuery.hover()
              + ", definition=" + typeQuery.definition());
      assertEquals("java.util.List", typeDefinition.owner());
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void resolvesInheritanceGenericsAccessOverloadsAndMethodReferences() {
    javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
    try (javax.tools.StandardJavaFileManager files =
        compiler.getStandardFileManager(null, java.util.Locale.ROOT, null)) {
      String inherited =
          """
          import java.util.*;
          class Parent<T extends CharSequence & Comparable<T>> {
            protected T inherited() { return null; }
            private void hidden() {}
            void packageOnly() {}
            Number covariant() { return 0; }
          }
          class Child extends Parent<String> {
            @Override Integer covariant() { return 0; }
            void test() { this.inh; }
          }
          """;
      int inheritedCursor = inherited.indexOf("this.inh") + "this.inh".length();
      EditorQueryResult inheritedResult =
          EditorQueries.query(
              compiler, files, "Child.java", inherited, inheritedCursor, 25);
      EditorCompletion inheritedMethod =
          inheritedResult.completions().stream()
              .filter(completion -> completion.label().equals("inherited"))
              .findFirst()
              .orElseThrow();
      assertTrue(
          inheritedMethod.detail().contains("java.lang.String"),
          "generic member was not substituted: " + inheritedMethod);
      assertTrue(
          inheritedResult.completions().stream()
              .noneMatch(completion -> completion.label().equals("hidden")),
          "private inherited member leaked into completion");

      String overloads =
          """
          class Overloads {
            String choose(int value) { return "int"; }
            String choose(Integer value) { return "boxed"; }
            String choose(String... value) { return "varargs"; }
            void test() {
              String a = choose(1);
              String b = choose(Integer.valueOf(1));
              java.util.function.IntFunction<String> ref = this::choose;
            }
          }
          """;
      SemanticResult overloadResult =
          JavacFrontend.analyze(
              "Overloads.java", overloads, java.util.List.of(), java.util.List.of(), 25);
      java.util.Set<String> references =
          overloadResult.symbols().stream()
              .filter(
                  symbol ->
                      symbol.role().equals("reference") && symbol.name().equals("choose"))
              .map(SemanticSymbol::symbolId)
              .collect(java.util.stream.Collectors.toSet());
      assertTrue(
          references.contains("<unnamed>|Overloads#choose(I)Ljava/lang/String;"),
          references.toString());
      assertTrue(
          references.contains(
              "<unnamed>|Overloads#choose(Ljava/lang/Integer;)Ljava/lang/String;"),
          references.toString());
      assertTrue(
          !overloadResult.diagnostics().stream()
              .anyMatch(diagnostic -> diagnostic.kind().equals("error")),
          overloadResult.diagnostics().toString());
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void parsesWorkspaceBatchWithoutAttribution() {
    WorkspaceParseResult result =
        JavacFrontend.parseWorkspace(
            java.util.List.of(
                new SourceInput(
                    "/workspace/demo/Model.java",
                    "package demo; public record Model(String value) {}"),
                new SourceInput(
                    "demo/Service.java",
                    "package demo; import java.util.List; class Service { List<Model> values; }")),
            25,
            false);

    assertEquals(2, result.files().size());
    assertEquals("/workspace/demo/Model.java", result.files().get(0).fileName());
    assertTrue(
        result.files().get(0).symbols().stream()
            .anyMatch(symbol -> symbol.qualifiedName().equals("demo.Model")),
        "batch parser missed record declaration: " + result.files().get(0).symbols());
    assertTrue(
        result.files().get(1).imports().stream()
            .anyMatch(imported -> imported.contains("java.util.List")),
        "batch parser missed import");
    assertTrue(
        result.files().stream()
            .flatMap(file -> file.diagnostics().stream())
            .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
        "valid batch produced parse errors");
    byte[] wire = WireEncoder.encode(result);
    assertEquals("JFB1", new String(wire, 0, 4, java.nio.charset.StandardCharsets.US_ASCII));
  }

  private static void parsesSupportedReleaseAndPreviewSyntax() {
    record Syntax(int release, String source) {}
    for (Syntax syntax :
        java.util.List.of(
            new Syntax(8, "interface Legacy { default int value() { return 1; } }"),
            new Syntax(11, "class Eleven { private void hidden() {} }"),
            new Syntax(17, "sealed interface Shape permits Circle {} final class Circle implements Shape {}"),
            new Syntax(21, "record Pair(String left, String right) {}"),
            new Syntax(25, "record Modern(int value) {}"))) {
      WorkspaceParseResult result =
          JavacFrontend.parseWorkspace(
              java.util.List.of(new SourceInput("Syntax.java", syntax.source())),
              syntax.release(),
              false);
      assertTrue(
          result.files().getFirst().diagnostics().stream()
              .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
          "release " + syntax.release() + " syntax failed");
    }
    WorkspaceParseResult preview =
        JavacFrontend.parseWorkspace(
            java.util.List.of(
                new SourceInput(
                    "Preview.java",
                    "import module java.base; class Preview { java.util.List<String> values; }")),
            25,
            true);
    assertTrue(
        preview.files().getFirst().diagnostics().stream()
            .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
        "Java 25 preview syntax failed");
  }

  private static void sessionUsesTheLatestUnsavedBuffer() {
    long session =
        SemanticSessions.create(
            java.util.List.of(),
            java.util.List.of(),
            java.util.List.of(),
            null,
            java.util.List.of(),
            25);
    try {
      SemanticResult first =
          SemanticSessions.analyze(session, "Overlay.java", "class Overlay { String before; }");
      SemanticResult second =
          SemanticSessions.analyze(session, "Overlay.java", "class Overlay { int after; }");

      assertTrue(
          first.symbols().stream().anyMatch(symbol -> symbol.name().equals("before")),
          "first overlay was not analyzed");
      assertTrue(
          second.symbols().stream().anyMatch(symbol -> symbol.name().equals("after")),
          "updated overlay was not analyzed");
      assertTrue(
          second.symbols().stream().noneMatch(symbol -> symbol.name().equals("before")),
          "session returned stale overlay symbols");
      SemanticSession implementation = SemanticSessions.get(session);
      SemanticSessions.analyze(session, "Stable.java", "class Stable {}");
      SemanticSessions.analyze(session, "Stable.java", "class Stable {}");
      assertEquals(3L, implementation.analysisCount());
      assertEquals(2, implementation.cachedDocumentCount());
      assertTrue(SemanticSessions.invalidate(session, "Overlay.java"), "session was not found");
      assertEquals(1, implementation.cachedDocumentCount());
      SemanticSessions.analyze(session, "Overlay.java", "class Overlay { int after; }");
      assertEquals(4L, implementation.analysisCount());
    } finally {
      assertTrue(SemanticSessions.destroy(session), "session was not destroyed");
      assertTrue(!SemanticSessions.destroy(session), "session was destroyed twice");
    }
  }

  private static void enforcesJpmsReadabilityExportsAndModuleIdentity() {
    try {
      java.nio.file.Path root = java.nio.file.Files.createTempDirectory("javac-frontend-jpms");
      java.nio.file.Path api = root.resolve("api");
      java.nio.file.Path modules = root.resolve("modules");
      java.nio.file.Files.createDirectories(api.resolve("demo/api"));
      java.nio.file.Files.writeString(
          api.resolve("module-info.java"), "module demo.api { exports demo.api; }");
      java.nio.file.Files.writeString(
          api.resolve("demo/api/Api.java"),
          "package demo.api; public final class Api { public static String value() { return \"ok\"; } }");
      javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
      int status =
          compiler.run(
              null,
              null,
              null,
              "-d",
              modules.resolve("demo.api").toString(),
              api.resolve("module-info.java").toString(),
              api.resolve("demo/api/Api.java").toString());
      assertEquals(0, status);

      java.nio.file.Path consumer = root.resolve("consumer");
      java.nio.file.Files.createDirectories(consumer.resolve("demo/consumer"));
      java.nio.file.Path moduleInfo = consumer.resolve("module-info.java");
      java.nio.file.Files.writeString(
          moduleInfo, "module demo.consumer { requires demo.api; }");
      java.nio.file.Files.writeString(
          consumer.resolve("demo/consumer/Helper.java"),
          "package demo.consumer; final class Helper { static String value() { return \"ok\"; } }");
      long session =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              moduleInfo,
              java.util.List.of(),
              25);
      try {
        SemanticResult descriptor =
            SemanticSessions.analyze(
                session, moduleInfo.toString(), java.nio.file.Files.readString(moduleInfo));
        assertTrue(
            descriptor.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            descriptor.diagnostics().toString());
        SemanticResult readable =
            SemanticSessions.analyze(
                session,
                consumer.resolve("demo/consumer/Consumer.java").toString(),
                "package demo.consumer; import demo.api.Api; class Consumer { String value = Api.value() + Helper.value(); }");
        assertTrue(
            readable.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            readable.diagnostics().toString());
        assertTrue(
            readable.symbols().stream()
                .anyMatch(symbol -> symbol.symbolId().startsWith("demo.api|")),
            readable.symbols().toString());
        assertTrue(
            readable.symbols().stream()
                .anyMatch(symbol -> symbol.symbolId().startsWith("demo.consumer|")),
            readable.symbols().toString());
        String sdkQuery =
            "package demo.consumer; class SdkUse { java.util.List<String> value = java.util.Collections.emptyList(); }";
        int sdkCursor = sdkQuery.indexOf("emptyList") + 2;
        EditorDefinition sdkDefinition =
            SemanticSessions.editorQuery(session, "SdkUse.java", sdkQuery, sdkCursor).definition();
        assertTrue(sdkDefinition != null, "modular SDK definition was not resolved");
        assertEquals("java.util.Collections", sdkDefinition.owner());
        assertEquals("emptyList", sdkDefinition.name());
      } finally {
        SemanticSessions.destroy(session);
      }

      java.nio.file.Files.writeString(moduleInfo, "module demo.consumer { }");
      long unreadable =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              moduleInfo,
              java.util.List.of(),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                unreadable,
                consumer.resolve("demo/consumer/Consumer.java").toString(),
                "package demo.consumer; import demo.api.Api; class Consumer { Api value; }");
        assertTrue(
            result.diagnostics().stream()
                .anyMatch(
                    diagnostic ->
                        diagnostic.kind().equals("error")
                            && diagnostic.message().contains("does not read")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(unreadable);
      }
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void selectsMultiReleaseJarApisByProjectRelease() {
    try {
      java.nio.file.Path root = java.nio.file.Files.createTempDirectory("javac-frontend-mrjar");
      java.nio.file.Path baseSource = root.resolve("base-source");
      java.nio.file.Path modernSource = root.resolve("modern-source");
      java.nio.file.Path baseClasses = root.resolve("base-classes");
      java.nio.file.Path modernClasses = root.resolve("modern-classes");
      java.nio.file.Files.createDirectories(baseSource.resolve("demo/mr"));
      java.nio.file.Files.createDirectories(modernSource.resolve("demo/mr"));
      java.nio.file.Files.writeString(
          baseSource.resolve("module-info.java"), "module demo.mr { exports demo.mr; }");
      java.nio.file.Files.writeString(
          baseSource.resolve("demo/mr/Feature.java"),
          "package demo.mr; public final class Feature { public static String base() { return \"base\"; } }");
      java.nio.file.Files.writeString(
          modernSource.resolve("demo/mr/Feature.java"),
          "package demo.mr; public final class Feature { public static String base() { return \"base\"; } public static String modern() { return \"modern\"; } }");
      javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "--release",
              "11",
              "-d",
              baseClasses.toString(),
              baseSource.resolve("module-info.java").toString(),
              baseSource.resolve("demo/mr/Feature.java").toString()));
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "--release",
              "17",
              "-d",
              modernClasses.toString(),
              modernSource.resolve("demo/mr/Feature.java").toString()));

      java.nio.file.Path jar = root.resolve("demo-mr.jar");
      try (var output =
          new java.util.jar.JarOutputStream(
              java.nio.file.Files.newOutputStream(jar),
              new java.util.jar.Manifest(
                  new java.io.ByteArrayInputStream(
                      "Manifest-Version: 1.0\nMulti-Release: true\n\n"
                          .getBytes(java.nio.charset.StandardCharsets.UTF_8))))) {
        addJarEntry(output, baseClasses.resolve("module-info.class"), "module-info.class");
        addJarEntry(output, baseClasses.resolve("demo/mr/Feature.class"), "demo/mr/Feature.class");
        addJarEntry(
            output,
            modernClasses.resolve("demo/mr/Feature.class"),
            "META-INF/versions/17/demo/mr/Feature.class");
      }
      java.nio.file.Path sourcesJar = root.resolve("demo-mr-sources.jar");
      try (var output =
          new java.util.jar.JarOutputStream(
              java.nio.file.Files.newOutputStream(sourcesJar),
              new java.util.jar.Manifest(
                  new java.io.ByteArrayInputStream(
                      "Manifest-Version: 1.0\nMulti-Release: true\n\n"
                          .getBytes(java.nio.charset.StandardCharsets.UTF_8))))) {
        addJarEntry(
            output, baseSource.resolve("demo/mr/Feature.java"), "demo/mr/Feature.java");
        addJarEntry(
            output,
            modernSource.resolve("demo/mr/Feature.java"),
            "META-INF/versions/17/demo/mr/Feature.java");
      }

      String use =
          "import demo.mr.Feature; class Use { String value = Feature.modern(); }";
      for (int release : java.util.List.of(11, 17)) {
        long session =
            SemanticSessions.create(
                java.util.List.of(),
                java.util.List.of(jar),
                java.util.List.of(),
                null,
                java.util.List.of("--add-modules", "ALL-MODULE-PATH"),
                release);
        try {
          SemanticResult result = SemanticSessions.analyze(session, "Use.java", use);
          boolean hasError =
              result.diagnostics().stream()
                  .anyMatch(diagnostic -> diagnostic.kind().equals("error"));
          assertTrue(
              hasError == (release == 11),
              "release " + release + " diagnostics: " + result.diagnostics());
        } finally {
          SemanticSessions.destroy(session);
        }
      }
      long editorSession =
          SemanticSessions.create(
              java.util.List.of(jar),
              java.util.List.of(),
              java.util.List.of(sourcesJar),
              null,
              java.util.List.of(),
              17);
      try {
        String editorSource =
            "import demo.mr.Feature; class Use { String value = Feature.modern(); }";
        int cursor = editorSource.indexOf("modern") + 2;
        EditorDefinition definition =
            SemanticSessions.editorQuery(editorSession, "Use.java", editorSource, cursor)
                .definition();
        assertTrue(definition != null, "multi-release definition was not resolved");
        assertTrue(
            definition.sourceName().contains("META-INF/versions/17"),
            definition.sourceName());
        assertTrue(definition.source().contains("modern()"), definition.source());
      } finally {
        SemanticSessions.destroy(editorSession);
      }
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void supportsTransitiveServicesQualifiedAccessAndCompilerOverrides() {
    try {
      java.nio.file.Path root = java.nio.file.Files.createTempDirectory("javac-frontend-jpms-full");
      java.nio.file.Path modules = root.resolve("modules");
      java.nio.file.Path base = root.resolve("base");
      java.nio.file.Files.createDirectories(base.resolve("demo/base"));
      java.nio.file.Files.createDirectories(base.resolve("demo/hidden"));
      java.nio.file.Files.writeString(
          base.resolve("module-info.java"), "module demo.base { exports demo.base; }");
      java.nio.file.Files.writeString(
          base.resolve("demo/base/Api.java"),
          "package demo.base; public final class Api { public static String value() { return \"ok\"; } }");
      java.nio.file.Files.writeString(
          base.resolve("demo/base/Service.java"),
          "package demo.base; public interface Service { String value(); }");
      java.nio.file.Files.writeString(
          base.resolve("demo/hidden/Hidden.java"),
          "package demo.hidden; public final class Hidden {}");
      javax.tools.JavaCompiler compiler = javax.tools.ToolProvider.getSystemJavaCompiler();
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "-d",
              modules.resolve("demo.base").toString(),
              base.resolve("module-info.java").toString(),
              base.resolve("demo/base/Api.java").toString(),
              base.resolve("demo/base/Service.java").toString(),
              base.resolve("demo/hidden/Hidden.java").toString()));

      java.nio.file.Path middle = root.resolve("middle");
      java.nio.file.Files.createDirectories(middle.resolve("demo/middle"));
      java.nio.file.Files.writeString(
          middle.resolve("module-info.java"),
          "module demo.middle { requires transitive demo.base; exports demo.middle; }");
      java.nio.file.Files.writeString(
          middle.resolve("demo/middle/Middle.java"),
          "package demo.middle; public final class Middle {}");
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "--module-path",
              modules.toString(),
              "-d",
              modules.resolve("demo.middle").toString(),
              middle.resolve("module-info.java").toString(),
              middle.resolve("demo/middle/Middle.java").toString()));

      java.nio.file.Path consumer = root.resolve("consumer");
      java.nio.file.Files.createDirectories(consumer.resolve("demo/consumer"));
      java.nio.file.Path descriptor = consumer.resolve("module-info.java");
      java.nio.file.Files.writeString(
          descriptor,
          """
          module demo.consumer {
            requires demo.middle;
            uses demo.base.Service;
            provides demo.base.Service with demo.consumer.Provider;
          }
          """);
      java.nio.file.Files.writeString(
          consumer.resolve("demo/consumer/Provider.java"),
          "package demo.consumer; public final class Provider implements demo.base.Service { public Provider() {} public String value() { return \"ok\"; } }");
      long session =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(),
              25);
      try {
        SemanticResult transitive =
            SemanticSessions.analyze(
                session,
                consumer.resolve("demo/consumer/Use.java").toString(),
                "package demo.consumer; import demo.base.Api; class Use { String value = Api.value(); }");
        assertTrue(
            transitive.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            transitive.diagnostics().toString());
        SemanticResult services =
            SemanticSessions.analyze(
                session, descriptor.toString(), java.nio.file.Files.readString(descriptor));
        assertTrue(
            services.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            services.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(session);
      }

      String hiddenUse =
          "package demo.consumer; import demo.hidden.Hidden; class HiddenUse { Hidden value; }";
      long inaccessible =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(),
              25);
      try {
        assertTrue(
            SemanticSessions.analyze(
                    inaccessible,
                    consumer.resolve("demo/consumer/HiddenUse.java").toString(),
                    hiddenUse)
                .diagnostics()
                .stream()
                .anyMatch(diagnostic -> diagnostic.kind().equals("error")),
            "non-exported package was accessible");
      } finally {
        SemanticSessions.destroy(inaccessible);
      }
      long exported =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(
                  "--add-exports", "demo.base/demo.hidden=demo.consumer"),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                exported,
                consumer.resolve("demo/consumer/HiddenUse.java").toString(),
                hiddenUse);
        assertTrue(
            result.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(exported);
      }

      java.nio.file.Files.writeString(descriptor, "module demo.consumer { }");
      long addedRead =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(
                  "--add-modules", "demo.base",
                  "--add-reads", "demo.consumer=demo.base"),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                addedRead,
                consumer.resolve("demo/consumer/ReadOverride.java").toString(),
                "package demo.consumer; import demo.base.Api; class ReadOverride { String value = Api.value(); }");
        assertTrue(
            result.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(addedRead);
      }

      java.nio.file.Path patchSource = root.resolve("patch-source/demo/patch");
      java.nio.file.Path patchClasses = root.resolve("patch-classes");
      java.nio.file.Files.createDirectories(patchSource);
      java.nio.file.Files.writeString(
          patchSource.resolve("Extra.java"),
          "package demo.patch; public final class Extra {}");
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "--module-path",
              modules.toString(),
              "--patch-module",
              "demo.base=" + patchSource.getParent().getParent(),
              "-d",
              patchClasses.toString(),
              patchSource.resolve("Extra.java").toString()));
      long patched =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(
                  "--add-modules", "demo.base",
                  "--add-reads", "demo.consumer=demo.base",
                  "--patch-module", "demo.base=" + patchClasses,
                  "--add-exports", "demo.base/demo.patch=demo.consumer"),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                patched,
                consumer.resolve("demo/consumer/PatchUse.java").toString(),
                "package demo.consumer; import demo.patch.Extra; class PatchUse { Extra value; }");
        assertTrue(
            result.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(patched);
      }

      java.nio.file.Path automaticSource = root.resolve("automatic/demo/automatic");
      java.nio.file.Path automaticClasses = root.resolve("automatic-classes");
      java.nio.file.Files.createDirectories(automaticSource);
      java.nio.file.Files.writeString(
          automaticSource.resolve("AutomaticApi.java"),
          "package demo.automatic; public final class AutomaticApi {}");
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "-d",
              automaticClasses.toString(),
              automaticSource.resolve("AutomaticApi.java").toString()));
      java.nio.file.Path automaticJar = root.resolve("unrelated-file-name-1.0.jar");
      try (var output =
          new java.util.jar.JarOutputStream(
              java.nio.file.Files.newOutputStream(automaticJar),
              new java.util.jar.Manifest(
                  new java.io.ByteArrayInputStream(
                      "Manifest-Version: 1.0\nAutomatic-Module-Name: demo.automatic.module\n\n"
                          .getBytes(java.nio.charset.StandardCharsets.UTF_8))))) {
        addJarEntry(
            output,
            automaticClasses.resolve("demo/automatic/AutomaticApi.class"),
            "demo/automatic/AutomaticApi.class");
      }
      java.nio.file.Files.writeString(
          descriptor, "module demo.consumer { requires demo.automatic.module; }");
      long automatic =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(automaticJar),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                automatic,
                consumer.resolve("demo/consumer/AutomaticUse.java").toString(),
                "package demo.consumer; import demo.automatic.AutomaticApi; class AutomaticUse { AutomaticApi value; }");
        assertTrue(
            result.diagnostics().stream()
                .noneMatch(diagnostic -> diagnostic.kind().equals("error")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(automatic);
      }

      java.nio.file.Path qualified = root.resolve("qualified");
      java.nio.file.Files.createDirectories(qualified.resolve("demo/qualified"));
      java.nio.file.Files.createDirectories(qualified.resolve("demo/internal"));
      java.nio.file.Files.writeString(
          qualified.resolve("module-info.java"),
          """
          module demo.qualified {
            exports demo.qualified to demo.friend;
            opens demo.internal to demo.friend;
          }
          """);
      java.nio.file.Files.writeString(
          qualified.resolve("demo/qualified/QualifiedApi.java"),
          "package demo.qualified; public final class QualifiedApi {}");
      java.nio.file.Files.writeString(
          qualified.resolve("demo/internal/Internal.java"),
          "package demo.internal; public final class Internal {}");
      assertEquals(
          0,
          compiler.run(
              null,
              null,
              null,
              "-d",
              modules.resolve("demo.qualified").toString(),
              qualified.resolve("module-info.java").toString(),
              qualified.resolve("demo/qualified/QualifiedApi.java").toString(),
              qualified.resolve("demo/internal/Internal.java").toString()));
      for (String consumerModule : java.util.List.of("demo.friend", "demo.stranger")) {
        java.nio.file.Files.writeString(
            descriptor,
            "module " + consumerModule + " { requires demo.qualified; }");
        long qualifiedSession =
            SemanticSessions.create(
                java.util.List.of(),
                java.util.List.of(modules),
                java.util.List.of(consumer),
                descriptor,
                java.util.List.of(),
                25);
        try {
          SemanticResult result =
              SemanticSessions.analyze(
                  qualifiedSession,
                  consumer.resolve("demo/consumer/QualifiedUse.java").toString(),
                  "package demo.consumer; import demo.qualified.QualifiedApi; class QualifiedUse { QualifiedApi value; }");
          boolean hasError =
              result.diagnostics().stream()
                  .anyMatch(diagnostic -> diagnostic.kind().equals("error"));
          assertTrue(
              hasError == consumerModule.equals("demo.stranger"),
              consumerModule + ": " + result.diagnostics());
        } finally {
          SemanticSessions.destroy(qualifiedSession);
        }
      }

      for (String splitModule : java.util.List.of("demo.split.a", "demo.split.b")) {
        java.nio.file.Path split = root.resolve(splitModule);
        java.nio.file.Files.createDirectories(split.resolve("demo/shared"));
        java.nio.file.Files.writeString(
            split.resolve("module-info.java"),
            "module " + splitModule + " { exports demo.shared; }");
        String type = splitModule.endsWith(".a") ? "FromA" : "FromB";
        java.nio.file.Files.writeString(
            split.resolve("demo/shared/" + type + ".java"),
            "package demo.shared; public final class " + type + " {}");
        assertEquals(
            0,
            compiler.run(
                null,
                null,
                null,
                "-d",
                modules.resolve(splitModule).toString(),
                split.resolve("module-info.java").toString(),
                split.resolve("demo/shared/" + type + ".java").toString()));
      }
      java.nio.file.Files.writeString(
          descriptor,
          "module demo.consumer { requires demo.split.a; requires demo.split.b; }");
      long split =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                split,
                descriptor.toString(),
                java.nio.file.Files.readString(descriptor));
        assertTrue(
            result.diagnostics().stream()
                .anyMatch(
                    diagnostic ->
                        diagnostic.kind().equals("error")
                            && diagnostic.message().contains("demo.shared")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(split);
      }

      java.nio.file.Path duplicateBase = root.resolve("duplicate-base");
      copyTree(modules.resolve("demo.base"), duplicateBase);
      java.nio.file.Files.writeString(descriptor, "module demo.consumer { requires demo.base; }");
      long duplicate =
          SemanticSessions.create(
              java.util.List.of(),
              java.util.List.of(modules, duplicateBase),
              java.util.List.of(consumer),
              descriptor,
              java.util.List.of(),
              25);
      try {
        SemanticResult result =
            SemanticSessions.analyze(
                duplicate,
                descriptor.toString(),
                java.nio.file.Files.readString(descriptor));
        assertTrue(
            result.diagnostics().stream()
                .anyMatch(
                    diagnostic ->
                        diagnostic.kind().equals("error")
                            && diagnostic.message().contains("demo.base")),
            result.diagnostics().toString());
      } finally {
        SemanticSessions.destroy(duplicate);
      }
    } catch (java.io.IOException exception) {
      throw new AssertionError(exception);
    }
  }

  private static void addJarEntry(
      java.util.jar.JarOutputStream output, java.nio.file.Path source, String name)
      throws java.io.IOException {
    output.putNextEntry(new java.util.jar.JarEntry(name));
    java.nio.file.Files.copy(source, output);
    output.closeEntry();
  }

  private static void copyTree(java.nio.file.Path source, java.nio.file.Path target)
      throws java.io.IOException {
    try (var paths = java.nio.file.Files.walk(source)) {
      for (java.nio.file.Path path : paths.toList()) {
        java.nio.file.Path destination = target.resolve(source.relativize(path));
        if (java.nio.file.Files.isDirectory(path)) {
          java.nio.file.Files.createDirectories(destination);
        } else {
          java.nio.file.Files.copy(path, destination);
        }
      }
    }
  }

  private static void attributesJdkAndSourceSymbols() {
    String source =
        "package demo; import java.util.List; class Example { List<String> names; }";
    SemanticResult result =
        JavacFrontend.analyze(
            "Example.java", source, java.util.List.of(), java.util.List.of(), 25);

    assertTrue(
        result.symbols().stream()
            .anyMatch(symbol -> symbol.qualifiedName().equals("java.util.List")),
        "java.util.List was not attributed");
    assertTrue(
        result.symbols().stream()
            .anyMatch(
                symbol ->
                    symbol.role().equals("declaration")
                        && symbol.qualifiedName().equals("demo.Example")),
        "source declaration was not attributed");
    SemanticSymbol listReference =
        result.symbols().stream()
            .filter(symbol -> symbol.qualifiedName().equals("java.util.List"))
            .findFirst()
            .orElseThrow();
    assertEquals(
        "List",
        source.substring(
            Math.toIntExact(listReference.start()), Math.toIntExact(listReference.end())));
    assertTrue(
        result.diagnostics().stream().noneMatch(diagnostic -> diagnostic.kind().equals("error")),
        "valid attributed source produced errors");
    assertTrue(
        new String(WireEncoder.encode(result), java.nio.charset.StandardCharsets.ISO_8859_1)
            .startsWith("JFS1"),
        "semantic wire format has the wrong version");
  }

  private static void parsesTopLevelAndNestedDeclarations() {
    String source =
        "package demo; public final class Example { record Nested(int value) {} }";
    ParseResult result =
        JavacFrontend.parse("Example.java", source);

    assertEquals("demo", result.packageName());
    assertEquals(2, result.types().size());
    TypeDeclaration example = result.types().get(0);
    assertEquals("class", example.kind());
    assertEquals("Example", example.name());
    assertEquals(
        "public final class Example { record Nested(int value) {} }",
        source.substring(Math.toIntExact(example.start()), Math.toIntExact(example.end())));
    assertEquals("record", result.types().get(1).kind());
    assertEquals("Nested", result.types().get(1).name());
    assertTrue(result.diagnostics().isEmpty(), "valid source produced diagnostics");
  }

  private static void reportsMalformedSourceWithoutThrowing() {
    ParseResult result = JavacFrontend.parse("Broken.java", "class Broken { void test( }");

    assertTrue(!result.diagnostics().isEmpty(), "malformed source produced no diagnostics");
    Diagnostic diagnostic = result.diagnostics().get(0);
    assertEquals("error", diagnostic.kind());
    assertTrue(diagnostic.start() >= 0, "diagnostic has no source position");
    assertTrue(!diagnostic.code().isBlank(), "diagnostic has no stable code");
  }

  private static void recognizesJava25Declarations() {
    ParseResult result =
        JavacFrontend.parse(
            "Modern.java",
            "sealed interface Shape permits Circle {} final class Circle implements Shape {}");

    assertEquals(2, result.types().size());
    assertEquals("interface", result.types().get(0).kind());
    assertEquals("class", result.types().get(1).kind());
  }

  private static void wireFormatIsVersionedAndDeterministic() {
    ParseResult result = JavacFrontend.parse("Example.java", "class Example {}");
    byte[] first = WireEncoder.encode(result);
    byte[] second = WireEncoder.encode(result);

    assertEquals('J', (char) first[0]);
    assertEquals('F', (char) first[1]);
    assertEquals('E', (char) first[2]);
    assertEquals('1', (char) first[3]);
    assertTrue(java.util.Arrays.equals(first, second), "wire encoding is not deterministic");
  }

  private static void assertEquals(Object expected, Object actual) {
    if (!expected.equals(actual)) {
      throw new AssertionError("expected <%s> but was <%s>".formatted(expected, actual));
    }
  }

  private static void assertTrue(boolean condition, String message) {
    if (!condition) {
      throw new AssertionError(message);
    }
  }
}
