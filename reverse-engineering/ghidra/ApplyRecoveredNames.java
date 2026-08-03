import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Parameter;
import ghidra.program.model.symbol.SourceType;

import java.nio.file.Files;
import java.nio.file.Path;

public class ApplyRecoveredNames extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 1) {
            throw new IllegalArgumentException("expected SYMBOL_MANIFEST");
        }
        int renamedFunctions = 0;
        int renamedParameters = 0;
        for (String line : Files.readAllLines(Path.of(arguments[0]))) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            String[] fields = line.split("\\t", -1);
            if (fields.length != 6) {
                throw new IllegalArgumentException("invalid symbol row: " + line);
            }
            Address address = toAddr(fields[0]);
            Function function = getFunctionAt(address);
            if (function == null) {
                println("NO_FUNCTION " + address + " " + fields[1]);
                continue;
            }
            function.setName(fields[1], SourceType.USER_DEFINED);
            renamedFunctions++;
            String[] names = fields[2].isBlank() ? new String[0] : fields[2].split(",", -1);
            Parameter[] parameters = function.getParameters();
            int nameIndex = 0;
            for (Parameter parameter : parameters) {
                if (parameter.isAutoParameter() || nameIndex >= names.length) {
                    continue;
                }
                if (!names[nameIndex].isBlank()) {
                    parameter.setName(names[nameIndex], SourceType.USER_DEFINED);
                    renamedParameters++;
                }
                nameIndex++;
            }
            setPlateComment(
                    address,
                    "Recovered subsystem: " + fields[3] + "\n"
                            + "Confidence: " + fields[4] + "\n"
                            + "Evidence: " + fields[5]);
        }
        println("RENAMED_FUNCTIONS " + renamedFunctions);
        println("RENAMED_PARAMETERS " + renamedParameters);
    }
}
