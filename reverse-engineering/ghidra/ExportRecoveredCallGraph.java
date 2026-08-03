import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

public class ExportRecoveredCallGraph extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 2) {
            throw new IllegalArgumentException("expected SYMBOL_MANIFEST OUTPUT_FILE");
        }
        List<String> rows = new ArrayList<>();
        rows.add("source_address\tsource_name\ttarget_address\ttarget_name");
        for (String line : Files.readAllLines(Path.of(arguments[0]))) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            String[] fields = line.split("\\t", -1);
            Function source = getFunctionAt(toAddr(fields[0]));
            if (source == null) {
                continue;
            }
            source.getCalledFunctions(monitor).stream()
                    .sorted(Comparator.comparing(Function::getEntryPoint))
                    .forEach(target -> rows.add(
                            source.getEntryPoint() + "\t" + source.getName() + "\t"
                                    + target.getEntryPoint() + "\t" + target.getName()));
        }
        Files.write(
                Path.of(arguments[1]),
                rows,
                StandardOpenOption.CREATE,
                StandardOpenOption.TRUNCATE_EXISTING);
        println("EXPORTED_CALL_EDGES " + (rows.size() - 1));
    }
}
