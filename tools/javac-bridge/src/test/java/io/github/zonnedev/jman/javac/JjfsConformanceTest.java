package io.github.zonnedev.jman.javac;

/** Golden conformance tests for the normative JJFS 1 specification. */
final class JjfsConformanceTest {
  private JjfsConformanceTest() {
  }

  static void run() {
    formatsTheLexicalFoundationAndBlocks();
    wrapsDeclarationsCallsAndFluentChains();
    eliminatesStaticAndWildcardImports();
    removesUnusedImportsAndShortensQualifiedTypes();
    ordersMembersWithoutChangingInitializationOrder();
    formatsModernListsAndMultilineTypeHeaders();
    formatsArrayInitializersAndTrailingCommas();
    ordersModuleDirectivesByCategory();
    normalizesDeclarationAnnotationsAndModifiers();
    insertsControlFlowBracesAndWrapsLongConditionsAndCalls();
    formatsLongLambdasTernariesLoopsAndResources();
    resolvesImportConflictsAndHonorsOnePreference();
    wrapsLongThrowsClausesWithoutDetachingTheBrace();
    formatsMultilineAnnotationsAndLongGenericHeaders();
    formatsRecordComponentsLikeParameters();
    preservesDisabledRegionsCommentsAndLiterals();
    rejectsMalformedFormatterDirectives();
  }

  private static void formatsTheLexicalFoundationAndBlocks() {
    assertFormat(
        "class Style{void run(){if(true){call(1,2);}else{}}void call(int left,int right){}}\n",
        """
        class Style {
          void run() {
            if (true) {
              call(1, 2);
            } else {
            }
          }

          void call(int left, int right) {
          }
        }
        """);
  }

  private static void wrapsDeclarationsCallsAndFluentChains() {
    assertFormat(
        """
        import java.util.Optional;
        class CustomerService {
          void create(String first,String second,String third,String fourth){consume(first,second,third,fourth);}
          String find(Optional<String> customer){return customer.map((value) -> value.trim()).filter((value) -> !value.isEmpty()).orElseThrow();}
          void consume(String first,String second,String third,String fourth){}
        }
        """,
        """
        import java.util.Optional;

        class CustomerService {
          void create(
            String first,
            String second,
            String third,
            String fourth
          ) {
            consume(first, second, third, fourth);
          }

          String find(Optional<String> customer) {
            return customer.map(value -> value.trim())
                .filter(value -> !value.isEmpty())
                .orElseThrow();
          }

          void consume(
            String first,
            String second,
            String third,
            String fourth
          ) {
          }
        }
        """);
  }

  private static void eliminatesStaticAndWildcardImports() {
    assertFormat(
        """
        package demo;
        import java.util.*;
        import static java.util.Objects.requireNonNull;
        final class Imports {
          List<String> copy(List<String> input){return new ArrayList<>(requireNonNull(input));}
        }
        """,
        """
        package demo;

        import java.util.ArrayList;
        import java.util.List;
        import java.util.Objects;

        final class Imports {
          List<String> copy(List<String> input) {
            return new ArrayList<>(Objects.requireNonNull(input));
          }
        }
        """);
  }

  private static void ordersMembersWithoutChangingInitializationOrder() {
    assertFormat(
        """
        class Members {
          private void hidden(){}
          private static final int PRIVATE_LIMIT=2;
          static int first=initialize(1);
          static {first++;}
          static int second=initialize(first);
          public void exposed(){}
          public void read(int identifier){}
          public void between(){}
          public void read(String identifier){}
          static Members restore(){return new Members();}
          Members(){}
          public static final int PUBLIC_LIMIT=1;
          static int initialize(int value){return value;}
        }
        """,
        """
        class Members {
          public static final int PUBLIC_LIMIT = 1;
          private static final int PRIVATE_LIMIT = 2;

          static int first = initialize(1);
          static {
            first++;
          }
          static int second = initialize(first);

          Members() {
          }

          static Members restore() {
            return new Members();
          }

          public void exposed() {
          }

          public void read(int identifier) {
          }

          public void read(String identifier) {
          }

          public void between() {
          }

          static int initialize(int value) {
            return value;
          }

          private void hidden() {
          }
        }
        """);
  }

  private static void removesUnusedImportsAndShortensQualifiedTypes() {
    assertFormat(
        """
        package demo;
        import java.lang.String;
        import java.util.Set;
        final class Names {
          java.util.List<String> names;
          java.util.Map.Entry<String,String> entry;
        }
        """,
        """
        package demo;

        import java.util.List;
        import java.util.Map;

        final class Names {
          List<String> names;
          Map.Entry<String, String> entry;
        }
        """);
  }

  private static void formatsModernListsAndMultilineTypeHeaders() {
    assertFormat(
        """
        enum Status {ACTIVE,PENDING,BLOCKED;}
        final class CustomerLifecycleSynchronizationService extends AbstractTransactionalCustomerLifecycleService implements CustomerActivationEventHandler,CustomerDeactivationEventHandler,CustomerDeletionEventHandler {}
        class AbstractTransactionalCustomerLifecycleService {}
        interface CustomerActivationEventHandler {}
        interface CustomerDeactivationEventHandler {}
        interface CustomerDeletionEventHandler {}
        """,
        """
        enum Status {
          ACTIVE,
          PENDING,
          BLOCKED,
          ;
        }

        final class CustomerLifecycleSynchronizationService
          extends AbstractTransactionalCustomerLifecycleService
          implements CustomerActivationEventHandler,
            CustomerDeactivationEventHandler,
            CustomerDeletionEventHandler
        {
        }

        class AbstractTransactionalCustomerLifecycleService {
        }

        interface CustomerActivationEventHandler {
        }

        interface CustomerDeactivationEventHandler {
        }

        interface CustomerDeletionEventHandler {
        }
        """);
  }

  private static void preservesDisabledRegionsCommentsAndLiterals() {
    String source =
        """
        class Preserve {
          // jjfs: off
          void aligned( ){ int x=  1; }
          // jjfs: on
          String literal(){return "do  not  rewrite";}
        }
        """;
    String expected =
        """
        class Preserve {
          // jjfs: off
          void aligned( ){ int x=  1; }
          // jjfs: on
          String literal() {
            return "do  not  rewrite";
          }
        }
        """;
    assertFormat(source, expected);
  }

  private static void insertsControlFlowBracesAndWrapsLongConditionsAndCalls() {
    assertFormat(
        """
        class Flow {
          void evaluate(){if(firstConditionWithLongName()&&secondConditionWithLongName()&&thirdConditionWithLongName())activate();else reject();consume("first-argument-with-a-deliberately-long-value","second-argument-with-a-deliberately-long-value","third-argument-with-a-deliberately-long-value","fourth-argument-with-a-deliberately-long-value");}
          boolean firstConditionWithLongName(){return true;}
          boolean secondConditionWithLongName(){return true;}
          boolean thirdConditionWithLongName(){return true;}
          void activate(){}
          void reject(){}
          void consume(String first,String second,String third,String fourth){}
        }
        """,
        """
        class Flow {
          void evaluate() {
            if (
              firstConditionWithLongName()
              && secondConditionWithLongName()
              && thirdConditionWithLongName()
            ) {
              activate();
            } else {
              reject();
            }
            consume(
              "first-argument-with-a-deliberately-long-value",
              "second-argument-with-a-deliberately-long-value",
              "third-argument-with-a-deliberately-long-value",
              "fourth-argument-with-a-deliberately-long-value"
            );
          }

          boolean firstConditionWithLongName() {
            return true;
          }

          boolean secondConditionWithLongName() {
            return true;
          }

          boolean thirdConditionWithLongName() {
            return true;
          }

          void activate() {
          }

          void reject() {
          }

          void consume(
            String first,
            String second,
            String third,
            String fourth
          ) {
          }
        }
        """);
  }

  private static void formatsArrayInitializersAndTrailingCommas() {
    assertFormat(
        """
        class Arrays {
          int[] primes={2,3,5};
          String[] generated={createFirstCustomerName(),createSecondCustomerName(),createThirdCustomerName()};
          String createFirstCustomerName(){return "first";}
          String createSecondCustomerName(){return "second";}
          String createThirdCustomerName(){return "third";}
        }
        """,
        """
        class Arrays {
          int[] primes = {2, 3, 5};
          String[] generated = {
            createFirstCustomerName(),
            createSecondCustomerName(),
            createThirdCustomerName(),
          };

          String createFirstCustomerName() {
            return "first";
          }

          String createSecondCustomerName() {
            return "second";
          }

          String createThirdCustomerName() {
            return "third";
          }
        }
        """);
  }

  private static void ordersModuleDirectivesByCategory() {
    assertFormat(
        "module demo { uses demo.Service; exports demo.zeta; requires java.sql; exports demo.alpha; requires java.logging; }\n",
        """
        module demo {
          requires java.logging;
          requires java.sql;

          exports demo.alpha;
          exports demo.zeta;

          uses demo.Service;
        }
        """);
  }

  private static void normalizesDeclarationAnnotationsAndModifiers() {
    assertFormat(
        "@Deprecated @SuppressWarnings(\"removal\") final public class Modifiers { static private final int VALUE=1; @Deprecated@SuppressWarnings(\"unused\") final private String name=\"\"; synchronized public void run(){} }\n",
        """
        @Deprecated
        @SuppressWarnings("removal")
        public final class Modifiers {
          private static final int VALUE = 1;

          @Deprecated
          @SuppressWarnings("unused")
          private final String name = "";

          public synchronized void run() {
          }
        }
        """);
  }

  private static void formatsLongLambdasTernariesLoopsAndResources() {
    assertFormat(
        """
        import java.io.InputStream;
        import java.io.OutputStream;
        import java.util.Optional;
        class Layout {
          String related(Optional<String> customer){return customer.map(value -> findRelatedCustomerUsingLongRepositoryMethod(value.trim())).orElseThrow();}
          String status(boolean active,boolean paired,boolean blocked){return active&&paired&&!blocked?"active-customer-with-payment-method":"inactive-or-blocked-customer";}
          void copy(InputStream input,OutputStream output)throws Exception{try(input;output){for(int indexWithAnIntentionallyLongDescriptiveName=0;indexWithAnIntentionallyLongDescriptiveName<customersWithLongDescriptiveName().length;indexWithAnIntentionallyLongDescriptiveName++)consume(customersWithLongDescriptiveName()[indexWithAnIntentionallyLongDescriptiveName]);}}
          String findRelatedCustomerUsingLongRepositoryMethod(String value){return value;}
          String[] customersWithLongDescriptiveName(){return new String[]{};}
          void consume(String value){}
        }
        """,
        """
        import java.io.InputStream;
        import java.io.OutputStream;
        import java.util.Optional;

        class Layout {
          String related(Optional<String> customer) {
            return customer.map(value -> {
              return findRelatedCustomerUsingLongRepositoryMethod(value.trim());
            })
                .orElseThrow();
          }

          String status(boolean active, boolean paired, boolean blocked) {
            return active && paired && !blocked
              ? "active-customer-with-payment-method"
              : "inactive-or-blocked-customer";
          }

          void copy(InputStream input, OutputStream output) throws Exception {
            try (
              input;
              output
            ) {
              for (
                int indexWithAnIntentionallyLongDescriptiveName = 0;
                indexWithAnIntentionallyLongDescriptiveName < customersWithLongDescriptiveName().length;
                indexWithAnIntentionallyLongDescriptiveName++
              ) {
                consume(customersWithLongDescriptiveName()[indexWithAnIntentionallyLongDescriptiveName]);
              }
            }
          }

          String findRelatedCustomerUsingLongRepositoryMethod(String value) {
            return value;
          }

          String[] customersWithLongDescriptiveName() {
            return new String[] {};
          }

          void consume(String value) {
          }
        }
        """);
  }

  private static void resolvesImportConflictsAndHonorsOnePreference() {
    assertFormat(
        """
        class Dates {
          java.sql.Date databaseDate;
          java.util.Date utilityDate;
        }
        """,
        """
        import java.sql.Date;

        class Dates {
          Date databaseDate;
          java.util.Date utilityDate;
        }
        """);
    assertFormat(
        """
        import java.util.Date; // jjfs: prefer-import
        class Dates {
          java.sql.Date databaseDate;
          Date utilityDate;
        }
        """,
        """
        import java.util.Date; // jjfs: prefer-import

        class Dates {
          java.sql.Date databaseDate;
          Date utilityDate;
        }
        """);
  }

  private static void wrapsLongThrowsClausesWithoutDetachingTheBrace() {
    assertFormat(
        """
        class Failures {
          void execute(String first,String second,String third) throws FirstExceptionWithAnIntentionallyLongName,SecondExceptionWithAnIntentionallyLongName,ThirdExceptionWithAnIntentionallyLongName {}
          static class FirstExceptionWithAnIntentionallyLongName extends Exception {}
          static class SecondExceptionWithAnIntentionallyLongName extends Exception {}
          static class ThirdExceptionWithAnIntentionallyLongName extends Exception {}
        }
        """,
        """
        class Failures {
          void execute(
            String first,
            String second,
            String third
          ) throws FirstExceptionWithAnIntentionallyLongName,
            SecondExceptionWithAnIntentionallyLongName,
            ThirdExceptionWithAnIntentionallyLongName {
          }

          static class FirstExceptionWithAnIntentionallyLongName extends Exception {
          }

          static class SecondExceptionWithAnIntentionallyLongName extends Exception {
          }

          static class ThirdExceptionWithAnIntentionallyLongName extends Exception {
          }
        }
        """);
  }

  private static void formatsMultilineAnnotationsAndLongGenericHeaders() {
    assertFormat(
        """
        @interface Rule { String regexp(); String message(); }
        class GenericFormatting {
          void validate(@Rule(
          regexp="[A-Z]{4}-[0-9]{8}", message="The external customer identifier must match the required uppercase format"
          ) String identifier){}
          public <T extends VeryLongCustomerAggregateName & Serializable,R extends VeryLongCustomerTransformationResultName & Comparable<R>> R transform(T customer){return null;}
          interface VeryLongCustomerAggregateName {}
          interface VeryLongCustomerTransformationResultName {}
          interface Serializable {}
        }
        """,
        """
        @interface Rule {
          String regexp();

          String message();
        }

        class GenericFormatting {
          public <
            T extends VeryLongCustomerAggregateName & Serializable,
            R extends VeryLongCustomerTransformationResultName & Comparable<R>
          >
          R transform(
            T customer
          ) {
            return null;
          }

          void validate(
            @Rule(
              regexp = "[A-Z]{4}-[0-9]{8}",
              message = "The external customer identifier must match the required uppercase format"
            )
            String identifier
          ) {
          }

          interface VeryLongCustomerAggregateName {
          }

          interface VeryLongCustomerTransformationResultName {
          }

          interface Serializable {
          }
        }
        """);
  }

  private static void formatsRecordComponentsLikeParameters() {
    assertFormat(
        "record Customer(String identifier,String displayName,String emailAddress,String telephoneNumber){}\n",
        """
        record Customer(
          String identifier,
          String displayName,
          String emailAddress,
          String telephoneNumber
        ) {
        }
        """);
  }

  private static void assertFormat(String source, String expected) {
    FormatResult first =
        JavaFormatter.format(
            "Conformance.java", source, java.util.List.of(), java.util.List.of(), 25);
    if (!first.diagnostics().isEmpty()) {
      throw new AssertionError(first.diagnostics().toString());
    }
    if (!expected.equals(first.source())) {
      throw new AssertionError("expected <%s> but was <%s>".formatted(expected, first.source()));
    }
    FormatResult second =
        JavaFormatter.format(
            "Conformance.java", first.source(), java.util.List.of(), java.util.List.of(), 25);
    if (!first.equals(second)) {
      throw new AssertionError(
          "JJFS output is not idempotent:\n"
              + second.source()
              + "\nfirst diagnostics="
              + first.diagnostics()
              + "\nsecond diagnostics="
              + second.diagnostics()
              + "\nfirst length="
              + first.source().length()
              + ", second length="
              + second.source().length());
    }
  }

  private static void rejectsMalformedFormatterDirectives() {
    assertDirectiveError("class Broken {\n  // jjfs: off\n  void run( ) {}\n}\n");
    assertDirectiveError(
        "import java.util.Date; // jjfs: prefer-import\nclass Broken { Date value; }\n");
    assertDirectiveError("class Broken { // jjfs: off because generated\n}\n");
  }

  private static void assertDirectiveError(String source) {
    FormatResult result =
        JavaFormatter.format("Broken.java", source, java.util.List.of(), java.util.List.of(), 25);
    if (result.diagnostics().size() != 1
        || !result.diagnostics().get(0).code().equals("jjfs.err.directive")) {
      throw new AssertionError(result.diagnostics().toString());
    }
    if (!result.source().equals(source)) {
      throw new AssertionError("a directive error changed the source");
    }
  }
}
