package io.github.zonnedev.jman.runner;

import java.io.PrintWriter;
import java.io.StringWriter;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;
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
    private final Map<String, List<Interval>> childIntervals = new ConcurrentHashMap<>();

    @Override
    public void executionStarted(TestIdentifier identifier) {
        if (!isTestOrContainer(identifier)) return;
        startedAt.put(identifier.getUniqueId(), System.nanoTime());
        emit(identifier, reason(identifier, "started"), null, null, null);
    }

    @Override
    public void executionSkipped(TestIdentifier identifier, String reason) {
        if (!isTestOrContainer(identifier)) return;
        emit(identifier, reason(identifier, "finished"), "skipped", reason, null);
    }

    @Override
    public void executionFinished(TestIdentifier identifier, TestExecutionResult result) {
        if (!isTestOrContainer(identifier)) return;
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
                reason(identifier, "finished"),
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
        long durationNanos = 0;
        long childDurationNanos = 0;
        if (status != null) {
            Long started = startedAt.remove(identifier.getUniqueId());
            long finished = System.nanoTime();
            long startedNanos = started == null ? finished : started;
            durationNanos = finished - startedNanos;
            childDurationNanos = coveredChildDuration(identifier.getUniqueId());
            Interval interval = new Interval(startedNanos, finished);
            identifier
                    .getParentId()
                    .ifPresent(
                            parent ->
                                    childIntervals
                                            .computeIfAbsent(
                                                    parent,
                                                    ignored ->
                                                            Collections.synchronizedList(
                                                                    new ArrayList<>()))
                                            .add(interval));
        }
        long durationMillis = durationNanos / 1_000_000;
        long lifecycleMillis = Math.max(0, durationNanos - childDurationNanos) / 1_000_000;
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
            field(json, "durationNanos", Long.toString(durationNanos), false);
            field(json, "durationMillis", Long.toString(durationMillis), false);
            if (identifier.isContainer() && !identifier.isTest()) {
                field(
                        json,
                        "lifecycleNanos",
                        Long.toString(Math.max(0, durationNanos - childDurationNanos)),
                        false);
                field(json, "lifecycleMillis", Long.toString(lifecycleMillis), false);
            }
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

    private static boolean isTestOrContainer(TestIdentifier identifier) {
        return identifier.isTest() || identifier.isContainer();
    }

    private long coveredChildDuration(String identifier) {
        List<Interval> intervals = childIntervals.remove(identifier);
        if (intervals == null || intervals.isEmpty()) return 0;
        List<Interval> ordered;
        synchronized (intervals) {
            ordered = new ArrayList<>(intervals);
        }
        ordered.sort(Comparator.comparingLong(interval -> interval.started));
        long covered = 0;
        long started = ordered.get(0).started;
        long finished = ordered.get(0).finished;
        for (int index = 1; index < ordered.size(); index++) {
            Interval interval = ordered.get(index);
            if (interval.started > finished) {
                covered += finished - started;
                started = interval.started;
            }
            finished = Math.max(finished, interval.finished);
        }
        return covered + finished - started;
    }

    private static String reason(TestIdentifier identifier, String phase) {
        return identifier.isTest() ? "test-" + phase : "container-" + phase;
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
            return new Source(className, className, null);
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

    private static final class Interval {
        private final long started;
        private final long finished;

        private Interval(long started, long finished) {
            this.started = started;
            this.finished = finished;
        }
    }
}
