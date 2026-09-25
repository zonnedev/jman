# JMAN Java Formatting Style 1

JMAN Java Formatting Style 1 (JJFS 1) is the canonical Java source format
produced by `jman fmt` and JMAN's editor integrations. This document is the
normative specification for the style. Examples marked **required** describe
formatter output; explanatory examples are non-normative.

JJFS is deliberately opinionated and has no style configuration. Given the
same valid source and resolved project model, the command line, VS Code, and
Neovim must produce identical output.

## Contract

JJFS output is:

- deterministic and idempotent;
- UTF-8 text with LF line endings and exactly one final newline;
- produced for the complete compilation unit, never for an isolated range;
- atomic per file: a file is replaced only after the complete result reparses
  and passes the formatter's safety validation;
- aware of the project's Java release, class path, module path, and source
  path;
- conservative when source cannot be parsed or a semantic rewrite cannot be
  resolved safely.

An error in one file does not prevent other valid files from being formatted.
Editor formatting returns no edits for a file that fails safety validation.

## Lexical foundation

### Indentation and whitespace

JJFS uses two spaces per block or ordinary continuation level. Tabs are not
emitted. Trailing whitespace is removed.

```java
if (customer.isActive()) {
  activate(customer);
}
```

Binary, assignment, comparison, logical, and ternary operators have one space
on each side. Dots, method references, unary operators, increment, and
decrement do not. Commas are followed by one space when content follows.
There is no padding immediately inside parentheses, brackets, or generic angle
brackets.

```java
if (value == 1) {
  service.execute(first, second);
}

Map<String, List<Customer>> customers;
Customer customer = (Customer) value;
```

Control-flow keywords have one space before `(`. Method, constructor, and
declaration names do not. Arrays use `Type[]`; varargs use `Type... values`.

### Width

The structural line limit is 140 displayed columns. JJFS must use a clean
syntactic break when one is available. A line may exceed the limit only for:

- an unbreakable identifier or qualified name;
- a package or import declaration;
- a preserved literal, text block, or comment;
- a disabled formatting region;
- a case where wrapping would leave an assignment operator dangling; or
- syntax with no semantics-preserving legal break.

Such unavoidable lines are canonical and must not make `jman fmt --check`
fail repeatedly.

### Literal spelling

String, character, numeric, and text-block tokens are opaque. JJFS does not
change numeric separators, bases, suffixes, escape sequences, Unicode escapes,
or literal forms. Text-block contents, indentation, blank lines, and closing
delimiter placement are preserved because they can affect the runtime value.

## Braces and blocks

Opening braces normally remain on the declaration or statement line. Every
brace-delimited body is multiline, including an empty body. Control-flow
braces are mandatory; JJFS inserts them around an unbraced body.

```java
void activate(Customer customer) {
  if (customer.isActive()) {
    activationService.activate(customer);
  }
}

void doNothing() {
}
```

Connected clauses remain attached to the preceding closing brace.

```java
try {
  repository.save(customer);
} catch (PersistenceException exception) {
  throw new CustomerSaveException(exception);
} finally {
  transaction.close();
}
```

`else`, `catch`, `finally`, and the `while` belonging to `do` follow this rule.

The sole brace-placement exception is a multiline type declaration. Its
opening brace occupies a line aligned with the declaration.

```java
final class CustomerLifecycleSynchronizationService
  extends AbstractTransactionalCustomerLifecycleService
  implements CustomerActivationEventHandler,
    CustomerDeactivationEventHandler,
    CustomerDeletionEventHandler
{
}
```

Methods and constructors keep `{` attached to the final header line.

## Line wrapping

### Parameters

A method or constructor declaration stays inline when it has at most three
parameters and fits within 140 columns. More than three parameters, an
overlong declaration, or a parameter that is itself multiline produces one
parameter per line. The closing `)` aligns with the declaration.

```java
Customer create(
  CustomerId id,
  CustomerName name,
  BillingAddress billingAddress,
  PaymentMethod paymentMethod
) {
  // ...
}
```

Short parameter annotations remain beside the parameter. A multiline
annotation moves above its parameter and uses one argument per line.

```java
Customer create(
  @NotNull CustomerId id,
  @Pattern(
    regexp = POSTAL_CODE_PATTERN,
    message = "The postal code is invalid"
  )
  String postalCode,
  @Valid BillingAddress address,
  PaymentMethod paymentMethod
) {
  // ...
}
```

Record components use the same rule. Compact record constructors are never
expanded into canonical constructors.

### Arguments

Argument count alone does not force a call to wrap. A call wraps when it
exceeds 140 columns, contains an already multiline argument, or contains a
combination of structurally complex arguments that is difficult to scan. A
wrapped call uses one argument per line and aligns its closing delimiter with
the containing statement.

```java
var customer = customerFactory.create(
  customerId,
  customerName,
  billingAddress,
  paymentMethod
);
```

### Fluent chains

A short chain remains inline. The receiver and first invocation always stay on
the initial line when a chain wraps; every subsequent invocation starts on its
own line with a four-space chain continuation.

```java
var customer = customerRepository.findById(id).orElseThrow();

var paired = customerRepository.findById(id)
    .map(customer -> customer.isPaired())
    .orElseThrow();
```

A chain stays inline when it has at most two calls and fits within 140 columns.
A chain of three or four structurally simple calls may stay inline when it
fits within 100 columns. A chain wraps when it:

- exceeds 140 columns;
- contains five or more calls;
- contains at least three calls and an explicit lambda, ternary, nested
  complex call, or other compound argument;
- contains an already multiline segment; or
- contains a block lambda.

Method references are structurally simple. The receiver is never left alone
after `return`, `=`, `->`, or another operator.

### Operators, conditions, and assignments

Wrapped non-assignment expressions put the operator at the beginning of the
continuation line.

```java
var eligible = customer.isActive()
  && customer.hasPaymentMethod()
  && !customer.isBlocked();
```

Short conditions up to 80 columns stay inline. A longer condition uses a symmetric
parenthesized form and aligns its operands.

```java
if (
  customer.isActive()
  && customer.hasPaymentMethod()
  && !customer.isBlocked()
) {
  activate(customer);
}
```

The same form applies to `while`. An assignment operator must never dangle:
the operator and the first meaningful token of its right-hand expression are
always on the same line. This rule takes precedence over the width limit.

```java
var customer = customerFactory.create(
  customerId,
  customerName,
  billingAddress
);
```

The following is forbidden:

```java
var customer =
  customerFactory.create(customerId);
```

### Ternaries

Simple ternaries stay inline. A long or structurally complex ternary uses
leading, vertically aligned `?` and `:`. Nested ternaries always wrap. JJFS
does not convert a ternary into `if`/`else`.

```java
var status = active ? Status.ACTIVE : Status.INACTIVE;

var status = customer.isActive() && customer.hasPaymentMethod()
  ? Status.ACTIVE
  : Status.INACTIVE;
```

### Lambdas

Optional parentheses around one inferred parameter are removed. Parentheses
remain when required by the language.

```java
.filter(customer -> customer.isActive())
```

A short expression lambda remains inline. An expression lambda that would
require a line break becomes a block. The formatter uses the resolved
functional descriptor to emit `return expression;` for a value-producing
lambda and `expression;` for a void-producing lambda. It performs this rewrite
only when resolution proves that the target remains unchanged.

```java
var related = customers.stream()
    .map(customer -> {
      return customerRepository.findRelatedCustomer(customer.identifier());
    })
    .toList();
```

### Loops and resources

Short loop headers remain inline. Long traditional `for` headers put
initialization, condition, and update on separate lines. Long enhanced loops
separate the declaration from the iterable expression.

```java
for (
  int index = 0;
  index < customers.size();
  index++
) {
  process(customers.get(index));
}
```

A try-with-resources statement stays inline with one short resource. Multiple
or long resources use one resource per line, with no final semicolon.

```java
try (
  var input = Files.newInputStream(sourcePath);
  var output = Files.newOutputStream(destinationPath)
) {
  input.transferTo(output);
}
```

### Type headers, generics, and throws

`extends`, `implements`, and `permits` do not wrap merely because they contain
several types. When the complete declaration exceeds 140 columns, each clause
uses a continuation line, keeps the first type beside the keyword, and indents
subsequent types one additional level.

```java
sealed interface CustomerEvent
  permits CustomerCreated,
    CustomerActivated,
    CustomerDeleted
{
}
```

Type-parameter count alone does not trigger wrapping. A genuinely overlong
type-parameter list uses a symmetric vertical form with one parameter per
line. Intersection bounds use leading `&`.

```java
public <T extends Customer & Serializable, R extends CustomerResult & Comparable<R>>
R transform(T customer) {
  // ...
}
```

When even that declaration cannot fit, the type parameters become vertical.
Long `throws` clauses keep the first exception beside `throws`, put subsequent
exceptions on separate continuation lines, preserve exception order, and keep
the method brace attached to the final exception.

## Imports

Imports form one uninterrupted lexicographically sorted block. JJFS removes
duplicates, unused imports, `java.lang` imports, and imports from the current
package. Wildcards are expanded to the referenced types.

Static imports are forbidden. JJFS resolves every imported static member,
qualifies its use with the owning top-level type, replaces the static import
with an ordinary owner import, and verifies that overload resolution is
unchanged.

```java
import java.util.Objects;
import org.assertj.core.api.Assertions;

Objects.requireNonNull(customer);
Assertions.assertThat(customer).isNotNull();
```

Unnecessary fully qualified type names become imports. Nested types retain
their top-level owner qualification:

```java
import java.util.Map;

Map.Entry<String, Customer> entry;
```

When used types share a simple name, the lexicographically first qualified name
receives the import and the others remain fully qualified. A source file may
override that arbitrary choice narrowly:

```java
import java.util.Date; // jjfs: prefer-import
```

The directive is valid only for an explicit, used, non-static type import that
participates in a simple-name conflict. Competing or stale directives are
formatter errors. It cannot override Java resolution or another JJFS rule.

## Declaration order

Members use this structural order:

1. JLS compile-time constants;
2. static fields and static initializer blocks;
3. instance fields and instance initializer blocks;
4. constructors;
5. static factories;
6. public methods;
7. protected methods;
8. package-private methods;
9. private methods;
10. nested types.

Compile-time constants are ordered by visibility—public, protected,
package-private, private—while retaining authored order within a visibility.
Other fields and initializer blocks retain their relative execution order.

A static method is a factory when its resolved return type is the enclosing
type or a compatible subtype. Factories retain authored order. Ordinary
methods are ordered by visibility and retain authored order within a
visibility. Same-visibility overloads are kept together at the first
overload's position. A private overload remains in the private section.

Nested types always appear last and are ordered internally by visibility.
Comments and Javadocs move with their declaration.

Related fields have no blank line between them. Different field groups and
member categories have exactly one. Constructors, methods—including
overloads—and nested types have exactly one blank line between them.

## Annotations and modifiers

Declaration annotations use one line each and retain authored order. Type-use
annotations never move. Modifiers use conventional Java order:

```text
public protected private
abstract default
static
sealed non-sealed
final
transient volatile
synchronized
native
strictfp
```

Multiline annotations use one argument per line. Repeatable annotations and
annotation elements are not reordered.

## Arrays, enums, switches, and declarations

Outside syntax that requires a combined declaration, JJFS emits one variable
per declaration. A `for` initializer may retain multiple variables.

Short array initializers stay inline. Long or complex arrays use one element
per line. Every non-empty multiline construct for which Java permits a
trailing comma receives one; inline constructs do not.

```java
int[] primes = {2, 3, 5};

int[] primes = {
  2,
  3,
  5,
};
```

Enum constants always use one line each and a trailing comma. If members
follow, the required semicolon follows the trailing comma on its own token
position.

```java
enum Status {
  ACTIVE,
  PENDING,
  BLOCKED,
  ;

  public boolean isTerminal() {
    return this == BLOCKED;
  }
}
```

JJFS preserves arrow-switch versus colon-switch semantics. Short arrow
expressions stay beside `->`; multiple statements use a block. A long
value-producing arrow expression becomes a verified block with `yield`.
JJFS never manufactures or removes fall-through, `break`, or `yield`.

## Comments and documentation

JJFS does not rewrite prose. Comment wording, wrapping, paragraph breaks, and
internal spacing remain authored. A long comment may exceed 140 columns.
Only indentation relative to surrounding code is normalized.

A short trailing comment remains attached. JJFS never aligns unrelated
trailing comments into columns. When a trailing comment cannot remain safely,
it moves immediately above its statement without changing its text.

Javadocs retain prose and tag order. JJFS may normalize only the outer
indentation, leading `*`, and tag indentation. Comments attached to reordered
members move with those members.

User-authored logical blank lines inside executable blocks are preserved,
multiple blank lines collapse to one, and blank lines immediately inside a
block are removed.

## Files and special compilation units

An initial license or file-header comment remains first. There are no leading
blank lines, exactly one blank line after a package declaration, exactly one
blank line between imports and the first type, and exactly one blank line
between top-level types. Top-level type order is preserved.

`package-info.java` keeps annotations and Javadocs attached to the package
declaration.

`module-info.java` orders directive categories as `requires`, `exports`,
`opens`, `uses`, and `provides`, with one blank line between non-empty
categories. Directives are alphabetized within a category. Provider
implementation order is preserved because it can be externally observable.
Combined modifiers use `requires static transitive`. Long target lists wrap
vertically. JJFS never infers module directives.

## Formatter directives

The only raw formatting escape hatch is a non-nesting disabled region:

```java
// jjfs: off
var deliberatelyAligned = Map.of(
    "short",      1,
    "muchLonger", 2
);
// jjfs: on
```

Content between the directives remains byte-for-byte unchanged. Directives
must be otherwise empty line comments. Missing, duplicated, nested, or
incorrectly ordered directives are formatter errors. `jman fmt --check`
ignores style inside a valid disabled region.

The only other directive is the import-conflict hint
`// jjfs: prefer-import` described above. JJFS has no general per-rule
suppression.

## Semantic preservation

JJFS may perform the explicitly specified structural normalizations, including
import qualification, member ordering, braces, declaration splitting, and
verified lambda or switch-arm blocks. It must not otherwise change the
program's meaning.

The formatter reparses its result before replacement. Semantic rewrites use
javac symbols rather than textual guesses and must preserve the relevant
resolved targets. If source is incomplete or attribution cannot establish a
safe rewrite, the file remains unchanged and the formatter reports why.
