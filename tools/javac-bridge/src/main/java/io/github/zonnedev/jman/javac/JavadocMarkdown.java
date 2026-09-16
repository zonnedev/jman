package io.github.zonnedev.jman.javac;

import java.util.ArrayList;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

final class JavadocMarkdown {
  private static final Pattern INLINE_CODE =
      Pattern.compile("\\{@(?:code|literal)\\s+([^}]*)}");
  private static final Pattern INLINE_LINK =
      Pattern.compile("\\{@link(?:plain)?\\s+([^}\\s]+)(?:\\s+([^}]+))?}");
  private static final Pattern HTML_CODE =
      Pattern.compile("<(?:code|tt)>(.*?)</(?:code|tt)>", Pattern.DOTALL);

  private JavadocMarkdown() {}

  static String render(String raw) {
    String text = raw.replace("\r\n", "\n").replace('\r', '\n');
    text = replace(INLINE_CODE, text, match -> "`" + match.group(1).trim() + "`");
    text = replace(
        INLINE_LINK,
        text,
        match -> "`" + (match.group(2) == null ? match.group(1) : match.group(2).trim()) + "`");
    text = replace(HTML_CODE, text, match -> "`" + match.group(1).trim() + "`");
    text = text.replaceAll("(?i)<p\\s*/?>", "\n\n")
        .replaceAll("(?i)<br\\s*/?>", "\n")
        .replaceAll("<[^>]+>", "");

    List<String> description = new ArrayList<>();
    List<String> parameters = new ArrayList<>();
    List<String> returns = new ArrayList<>();
    List<String> throwsTags = new ArrayList<>();
    List<String> metadata = new ArrayList<>();
    List<String> tags = new ArrayList<>();
    for (String line : text.split("\n")) {
      String trimmed = line.strip();
      if (trimmed.startsWith("@")) {
        tags.add(trimmed);
      } else if (!tags.isEmpty() && !trimmed.isEmpty()) {
        int last = tags.size() - 1;
        tags.set(last, tags.get(last) + " " + trimmed);
      } else if (!trimmed.isEmpty()) {
        description.add(trimmed);
      }
    }
    for (String tag : tags) {
      if (tag.startsWith("@param ")) parameters.add(tag.substring(7).strip());
      else if (tag.startsWith("@return ")) returns.add(tag.substring(8).strip());
      else if (tag.startsWith("@throws ") || tag.startsWith("@exception "))
        throwsTags.add(tag.substring(tag.indexOf(' ') + 1).strip());
      else if (tag.startsWith("@deprecated "))
        metadata.add("**Deprecated:** " + tag.substring(12).strip());
      else if (tag.startsWith("@since "))
        metadata.add("**Since:** " + tag.substring(7).strip());
      else if (tag.startsWith("@see "))
        metadata.add("**See:** `" + tag.substring(5).strip() + "`");
    }
    StringBuilder markdown = new StringBuilder(String.join("\n", description).strip());
    section(markdown, "Parameters", parameters);
    section(markdown, "Returns", returns);
    section(markdown, "Throws", throwsTags);
    for (String entry : metadata) {
      if (!markdown.isEmpty()) markdown.append("\n\n");
      markdown.append(entry);
    }
    return markdown.toString();
  }

  private static void section(StringBuilder output, String title, List<String> entries) {
    if (entries.isEmpty()) return;
    if (!output.isEmpty()) output.append("\n\n");
    output.append("**").append(title).append("**\n\n");
    for (String entry : entries) output.append("- ").append(entry).append('\n');
  }

  private static String replace(Pattern pattern, String input, Replacer replacer) {
    Matcher matcher = pattern.matcher(input);
    StringBuffer output = new StringBuffer();
    while (matcher.find()) {
      matcher.appendReplacement(output, Matcher.quoteReplacement(replacer.replace(matcher)));
    }
    matcher.appendTail(output);
    return output.toString();
  }

  private interface Replacer {
    String replace(Matcher matcher);
  }
}
