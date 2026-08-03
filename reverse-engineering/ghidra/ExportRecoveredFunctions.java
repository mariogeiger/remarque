import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.LinkedHashMap;
import java.util.Map;

public class ExportRecoveredFunctions extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 2) {
            throw new IllegalArgumentException("expected SYMBOL_MANIFEST OUTPUT_DIRECTORY");
        }
        Path outputDirectory = Path.of(arguments[1]);
        Files.createDirectories(outputDirectory);
        Map<String, StringBuilder> outputBySubsystem = new LinkedHashMap<>();
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        for (String line : Files.readAllLines(Path.of(arguments[0]))) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            String[] fields = line.split("\\t", -1);
            Function function = getFunctionAt(toAddr(fields[0]));
            if (function == null) {
                continue;
            }
            DecompileResults result = decompiler.decompileFunction(function, 300, monitor);
            StringBuilder output = outputBySubsystem.computeIfAbsent(fields[3], ignored -> new StringBuilder());
            output.append("/*\n * address: 0x").append(fields[0]).append("\n")
                    .append(" * confidence: ").append(fields[4]).append("\n")
                    .append(" * evidence: ").append(fields[5]).append("\n */\n");
            if (result.decompileCompleted()) {
                output.append(result.getDecompiledFunction().getC());
            } else {
                output.append("/* decompilation failed: ").append(result.getErrorMessage()).append(" */\n");
            }
            output.append("\n");
        }
        decompiler.dispose();
        for (Map.Entry<String, StringBuilder> entry : outputBySubsystem.entrySet()) {
            Files.writeString(
                    outputDirectory.resolve(entry.getKey() + ".c"),
                    entry.getValue().toString(),
                    StandardOpenOption.CREATE,
                    StandardOpenOption.TRUNCATE_EXISTING);
        }
        println("EXPORTED_SUBSYSTEMS " + outputBySubsystem.size());
    }
}
