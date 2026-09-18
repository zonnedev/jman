package io.github.zonnedev.jman.runner;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import org.junit.platform.engine.TestExecutionResult;
import org.junit.platform.engine.TestSource;
import org.junit.platform.engine.support.descriptor.ClassSource;
import org.junit.platform.engine.support.descriptor.MethodSource;
import org.junit.platform.launcher.TestExecutionListener;
import org.junit.platform.launcher.TestIdentifier;

/** Emits versioned, machine-readable test events without parsing console output. */
public final class JmanExecutionListener implements TestExecutionListener {
    private static final int PROTOCOL_VERSION = 3;
    private static final char RECORD_SEPARATOR = '\u001e';
    private final Map<String, Long> startedAt = new ConcurrentHashMap<>();

    @Override
    public void executionStarted(TestIdentifier identifier) {
        if (!identifier.isTest()) return;
        startedAt.put(identifier.getUniqueId(), System.nanoTime());
        emit(identifier, "test-started", null, null, null);
    }

    @Override
    public void executionSkipped(TestIdentifier identifier, String reason) {
        if (!identifier.isTest()) return;
        emit(identifier, "test-finished", "skipped", reason, null);
    }

    @Override
    public void executionFinished(TestIdentifier identifier, TestExecutionResult result) {
        if (!identifier.isTest()) return;
        String status;
        switch (result.getStatus()) {
            case SUCCESSFUL:
                status = "passed";
                break;
            case FAILED:
                status = "failed";
                break;
            case ABORTED:
                status = "errored";
                break;
            default:
                throw new IllegalStateException("Unknown JUnit status: " + result.getStatus());
        }
        Throwable failure = result.getThrowable().orElse(null);
        emit(
                identifier,
                "test-finished",
                status,
                failure == null ? null : failure.getMessage(),
                stackTrace(failure));
    }

    private void emit(
            TestIdentifier identifier,
            String reason,
            String status,
            String message,
            String details) {
        Source source = source(identifier.getSource().orElse(null));
        Long started = startedAt.remove(identifier.getUniqueId());
        long durationMillis = started == null ? 0 : (System.nanoTime() - started) / 1_000_000;
        StringBuilder json = new StringBuilder(512);
        json.append('{');
        field(json, "protocolVersion", Integer.toString(PROTOCOL_VERSION), false);
        field(json, "reason", reason, true);
        field(json, "id", identifier.getUniqueId(), true);
        field(json, "parentId", identifier.getParentId().orElse(null), true);
        field(json, "selector", source.selector, true);
        field(json, "className", source.className, true);
        field(json, "methodName", source.methodName, true);
        field(json, "displayName", identifier.getDisplayName(), true);
        if (status != null) {
            field(json, "status", status, true);
            field(json, "durationMillis", Long.toString(durationMillis), false);
            field(json, "message", message, true);
            field(json, "details", details, true);
        }
        json.append('}');
        synchronized (System.out) {
            System.out.print(RECORD_SEPARATOR);
            System.out.println(json);
            System.out.flush();
        }
    }

    private static Source source(TestSource source) {
        if (source instanceof MethodSource) {
            MethodSource method = (MethodSource) source;
            return new Source(
                    method.getClassName() + "#" + method.getMethodName(),
                    method.getClassName(),
                    method.getMethodName());
        }
        if (source instanceof ClassSource) {
            String className = ((ClassSource) source).getClassName();
            return new Source(className, className, "");
        }
        return new Source(null, null, null);
    }

    private static String stackTrace(Throwable failure) {
        if (failure == null) return null;
        StringWriter text = new StringWriter();
        failure.printStackTrace(new PrintWriter(text));
        return text.toString();
    }

    private static void field(StringBuilder json, String name, String value, boolean quoted) {
        if (json.length() > 1) json.append(',');
        string(json, name);
        json.append(':');
        if (value == null) {
            json.append("null");
        } else if (quoted) {
            string(json, value);
        } else {
            json.append(value);
        }
    }

    private static void string(StringBuilder json, String value) {
        json.append('"');
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            switch (character) {
                case '"': json.append("\\\""); break;
                case '\\': json.append("\\\\"); break;
                case '\b': json.append("\\b"); break;
                case '\f': json.append("\\f"); break;
                case '\n': json.append("\\n"); break;
                case '\r': json.append("\\r"); break;
                case '\t': json.append("\\t"); break;
                default:
                    if (character < 0x20) {
                        json.append(String.format("\\u%04x", (int) character));
                    } else {
                        json.append(character);
                    }
            }
        }
        json.append('"');
    }

    private static final class Source {
        private final String selector;
        private final String className;
        private final String methodName;

        private Source(String selector, String className, String methodName) {
            this.selector = selector;
            this.className = className;
            this.methodName = methodName;
        }
    }
}
