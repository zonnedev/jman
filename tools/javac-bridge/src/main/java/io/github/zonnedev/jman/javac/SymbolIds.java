package io.github.zonnedev.jman.javac;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import javax.lang.model.element.Element;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.ModuleElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import javax.lang.model.type.ArrayType;
import javax.lang.model.type.DeclaredType;
import javax.lang.model.type.ExecutableType;
import javax.lang.model.type.TypeKind;
import javax.lang.model.type.TypeMirror;
import javax.lang.model.util.Elements;
import javax.lang.model.util.Types;

final class SymbolIds {
  private SymbolIds() {}

  static String of(Elements elements, Types types, Element element) {
    String module = moduleName(elements, element);
    if (element instanceof TypeElement type) {
      return module + "|L" + binaryName(elements, type) + ";";
    }
    TypeElement owner = owner(element);
    if (owner == null) return module + "|" + element;
    String prefix = module + "|" + binaryName(elements, owner) + "#";
    if (element instanceof ExecutableElement executable) {
      ExecutableType erased = (ExecutableType) types.erasure(executable.asType());
      return prefix + executable.getSimpleName() + descriptor(elements, types, erased);
    }
    if (element instanceof VariableElement variable) {
      return prefix + variable.getSimpleName() + ":" + descriptor(elements, types, variable.asType());
    }
    return prefix + element.getSimpleName();
  }

  static String overrideFamily(Elements elements, Types types, Element element) {
    if (!(element instanceof ExecutableElement method)
        || method.getKind() == javax.lang.model.element.ElementKind.CONSTRUCTOR) {
      return "";
    }
    TypeElement owner = owner(method);
    if (owner == null) return "";
    List<ExecutableElement> overridden = new ArrayList<>();
    for (TypeMirror supertype : types.directSupertypes(owner.asType())) {
      if (!(types.asElement(supertype) instanceof TypeElement parent)) continue;
      for (Element member : elements.getAllMembers(parent)) {
        if (member instanceof ExecutableElement candidate
            && candidate.getSimpleName().contentEquals(method.getSimpleName())
            && elements.overrides(method, candidate, owner)) {
          overridden.add(candidate);
        }
      }
    }
    if (overridden.isEmpty()) return of(elements, types, method);
    return overridden.stream()
        .map(candidate -> overrideFamily(elements, types, candidate))
        .filter(value -> !value.isEmpty())
        .min(Comparator.naturalOrder())
        .orElseGet(() -> of(elements, types, method));
  }

  static String descriptor(Elements elements, Types types, Element element) {
    TypeMirror type = element.asType();
    if (element instanceof ExecutableElement) {
      type = types.erasure(type);
    }
    return descriptor(elements, types, type);
  }

  static String binaryOwner(Elements elements, Element element) {
    TypeElement owner = element instanceof TypeElement type ? type : owner(element);
    return owner == null ? "" : binaryName(elements, owner).replace('/', '.');
  }

  static String moduleName(Elements elements, Element element) {
    ModuleElement module = elements.getModuleOf(element);
    return module == null || module.isUnnamed() ? "<unnamed>" : module.getQualifiedName().toString();
  }

  static boolean isUnresolvedRecovery(Elements elements, Element element) {
    if (element.asType().getKind() != TypeKind.ERROR) return false;
    if (!(element instanceof TypeElement type)) return true;
    String qualified = type.getQualifiedName().toString();
    return moduleName(elements, element).equals("<unnamed>")
        && (qualified.isEmpty() || qualified.equals(type.getSimpleName().toString()));
  }

  private static String descriptor(Elements elements, Types types, TypeMirror type) {
    return switch (type.getKind()) {
      case BOOLEAN -> "Z";
      case BYTE -> "B";
      case SHORT -> "S";
      case INT -> "I";
      case LONG -> "J";
      case CHAR -> "C";
      case FLOAT -> "F";
      case DOUBLE -> "D";
      case VOID -> "V";
      case ARRAY -> "[" + descriptor(elements, types, ((ArrayType) type).getComponentType());
      case DECLARED -> {
        TypeElement declared = (TypeElement) ((DeclaredType) type).asElement();
        yield "L" + binaryName(elements, declared) + ";";
      }
      case EXECUTABLE -> {
        ExecutableType executable = (ExecutableType) type;
        StringBuilder value = new StringBuilder("(");
        for (TypeMirror parameter : executable.getParameterTypes()) {
          value.append(descriptor(elements, types, types.erasure(parameter)));
        }
        value.append(')');
        value.append(descriptor(elements, types, types.erasure(executable.getReturnType())));
        yield value.toString();
      }
      case TYPEVAR, WILDCARD, INTERSECTION ->
          descriptor(elements, types, types.erasure(type));
      case NONE -> "";
      default -> "Ljava/lang/Object;";
    };
  }

  private static String binaryName(Elements elements, TypeElement type) {
    return elements.getBinaryName(type).toString().replace('.', '/');
  }

  private static TypeElement owner(Element element) {
    Element current = element.getEnclosingElement();
    while (current != null) {
      if (current instanceof TypeElement type) return type;
      current = current.getEnclosingElement();
    }
    return null;
  }
}
