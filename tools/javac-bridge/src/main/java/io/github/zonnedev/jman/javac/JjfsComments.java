package io.github.zonnedev.jman.javac;

import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;

/** Conservative formatting for comment structure and provably plain prose. */
final class JjfsComments {
  private static final int PROSE_WIDTH = 100;
  private static final Pattern LIST_ITEM =
      Pattern.compile("(?:[-+*]|\\d+[.)])\\s+.*");

  private JjfsComments() {
  }

  static String format(String text, String kind, int indent, boolean trailing) {
    if (directive(text)) return text;
    if (trailing && (kind.equals("BLOCK") || kind.startsWith("JAVADOC"))) return text;
    if (kind.equals("JAVADOC_LINE")) return text;
    if (kind.equals("LINE")) return formatLine(text, "//", indent, trailing);
    if (kind.equals("BLOCK") && !text.contains("\n") && !text.contains("\r")) {
      if (trailing || indent * 2 + text.length() <= PROSE_WIDTH) return text;
    }
    if (kind.equals("JAVADOC_BLOCK") || kind.equals("JAVADOC") || kind.equals("BLOCK")) {
      return formatBlock(text, kind.startsWith("JAVADOC"), indent);
    }
    return text;
  }

  private static String formatLine(String text, String prefix, int indent, boolean trailing) {
    if (trailing || !text.startsWith(prefix)) return text;
    String content = text.substring(prefix.length()).strip();
    int width = PROSE_WIDTH - indent * 2 - prefix.length() - 1;
    if (content.isEmpty() || content.length() <= width || protectedContent(content)) return text;
    List<String> wrapped = wrap(content, width, "");
    String continuation = " ".repeat(indent * 2);
    return prefix + " " + String.join("\n" + continuation + prefix + " ", wrapped);
  }

  private static String formatBlock(String text, boolean javadoc, int indent) {
    String normalized = text.replace("\r\n", "\n").replace('\r', '\n');
    String opener = javadoc ? "/**" : "/*";
    int open = normalized.indexOf(opener);
    int close = normalized.lastIndexOf("*/");
    if (open < 0 || close < open + opener.length()) return text;

    String body = normalized.substring(open + opener.length(), close);
    List<String> lines = new ArrayList<>();
    for (String line : body.split("\n", -1)) lines.add(commentContent(line));
    trimBlankEdges(lines);
    lines =
        protectedBlockComment(body)
            ? collapseBlanks(lines)
            : formatBody(lines, PROSE_WIDTH - indent * 2 - 3, javadoc);

    String indentation = " ".repeat(indent * 2);
    StringBuilder formatted = new StringBuilder(opener);
    for (String line : lines) {
      formatted.append('\n').append(indentation).append(" *");
      if (!line.isEmpty()) formatted.append(' ').append(line);
    }
    formatted.append('\n').append(indentation).append(" */");
    return formatted.toString();
  }

  private static List<String> formatBody(List<String> source, int width, boolean javadoc) {
    List<String> formatted = new ArrayList<>();
    List<String> paragraph = new ArrayList<>();
    boolean protectedBlock = false;
    boolean tagsStarted = false;
    for (String line : source) {
      String stripped = line.strip();
      boolean fence = stripped.startsWith("```") || stripped.startsWith("~~~");
      boolean preStart = stripped.toLowerCase(java.util.Locale.ROOT).contains("<pre>");
      boolean preEnd = stripped.toLowerCase(java.util.Locale.ROOT).contains("</pre>");
      if (protectedBlock || fence || preStart) {
        flushParagraph(paragraph, formatted, width);
        formatted.add(line.stripTrailing());
        if (fence) protectedBlock = !protectedBlock;
        else if (preStart && !preEnd) protectedBlock = true;
        if (preEnd) protectedBlock = false;
        continue;
      }
      if (stripped.isEmpty()) {
        flushParagraph(paragraph, formatted, width);
        addBlank(formatted);
        continue;
      }
      if (javadoc && stripped.startsWith("@")) {
        flushParagraph(paragraph, formatted, width);
        if (!tagsStarted && !formatted.isEmpty() && !formatted.get(formatted.size() - 1).isEmpty()) {
          formatted.add("");
        }
        if (protectedContent(stripped)) formatted.add(line.stripTrailing());
        else formatted.addAll(wrap(stripped, width, "  "));
        tagsStarted = true;
        continue;
      }
      if (protectedContent(line)) {
        flushParagraph(paragraph, formatted, width);
        formatted.add(line.stripTrailing());
        continue;
      }
      paragraph.add(stripped);
    }
    flushParagraph(paragraph, formatted, width);
    trimBlankEdges(formatted);
    return collapseBlanks(formatted);
  }

  private static void flushParagraph(
      List<String> paragraph, List<String> formatted, int width) {
    if (paragraph.isEmpty()) return;
    formatted.addAll(wrap(String.join(" ", paragraph), width, ""));
    paragraph.clear();
  }

  private static List<String> wrap(String text, int width, String continuationPrefix) {
    List<String> lines = new ArrayList<>();
    StringBuilder line = new StringBuilder();
    for (String word : text.strip().split("\\s+")) {
      int limit = Math.max(20, width - (lines.isEmpty() ? 0 : continuationPrefix.length()));
      if (!line.isEmpty() && line.length() + 1 + word.length() > limit) {
        lines.add((lines.isEmpty() ? "" : continuationPrefix) + line);
        line.setLength(0);
      }
      if (!line.isEmpty()) line.append(' ');
      line.append(word);
    }
    if (!line.isEmpty()) lines.add((lines.isEmpty() ? "" : continuationPrefix) + line);
    return lines;
  }

  private static boolean protectedContent(String line) {
    String stripped = line.strip();
    String lower = stripped.toLowerCase(java.util.Locale.ROOT);
    return line.startsWith("  ")
        || stripped.isEmpty()
        || LIST_ITEM.matcher(stripped).matches()
        || stripped.matches("[-=*]{3,}")
        || stripped.startsWith("#")
        || stripped.startsWith(">")
        || stripped.startsWith("|")
        || stripped.endsWith("|")
        || stripped.startsWith("<")
        || lower.startsWith("noinspection")
        || lower.startsWith("language=")
        || lower.equals("region")
        || lower.equals("endregion")
        || lower.startsWith("checkstyle")
        || lower.startsWith("spotbugs")
        || lower.startsWith("sonar")
        || stripped.contains("://")
        || stripped.contains("{@")
        || stripped.contains("`")
        || stripped.contains("└")
        || stripped.contains("├")
        || stripped.contains("│")
        || stripped.contains("──")
        || stripped.contains(" -> ")
        || stripped.contains(" <- ");
  }

  private static String commentContent(String line) {
    String content = line.stripLeading();
    if (content.startsWith("*")) {
      content = content.substring(1);
      if (content.startsWith(" ")) content = content.substring(1);
    }
    return content.stripTrailing();
  }

  private static void addBlank(List<String> lines) {
    if (!lines.isEmpty() && !lines.get(lines.size() - 1).isEmpty()) lines.add("");
  }

  private static List<String> collapseBlanks(List<String> lines) {
    List<String> collapsed = new ArrayList<>();
    for (String line : lines) {
      if (line.isEmpty() && (collapsed.isEmpty() || collapsed.get(collapsed.size() - 1).isEmpty())) {
        continue;
      }
      collapsed.add(line);
    }
    return collapsed;
  }

  private static void trimBlankEdges(List<String> lines) {
    while (!lines.isEmpty() && lines.get(0).isBlank()) lines.remove(0);
    while (!lines.isEmpty() && lines.get(lines.size() - 1).isBlank()) {
      lines.remove(lines.size() - 1);
    }
  }

  private static boolean directive(String text) {
    String stripped = text.strip();
    return stripped.equals("// jjfs: off")
        || stripped.equals("// jjfs: on")
        || stripped.equals("// jjfs: prefer-import");
  }

  private static boolean protectedBlockComment(String body) {
    String lower = body.toLowerCase(java.util.Locale.ROOT);
    return lower.contains("copyright")
        || lower.contains("spdx-license-identifier")
        || lower.contains("licensed under")
        || lower.contains("generated by")
        || lower.contains("do not edit");
  }
}
