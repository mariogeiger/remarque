import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.Reference;
import java.util.LinkedHashMap;
import java.util.Map;

public class DecompileFunctionsReferencingAddress extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 1) {
            throw new IllegalArgumentException("expected ADDRESS");
        }
        Address target = toAddr(arguments[0]);
        Map<Address, Function> functions = new LinkedHashMap<>();
        for (Reference reference : currentProgram.getReferenceManager().getReferencesTo(target)) {
            Function function = getFunctionContaining(reference.getFromAddress());
            if (function != null) {
                functions.put(function.getEntryPoint(), function);
            }
        }

        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        for (Function function : functions.values()) {
            println("\n===== FUNCTION " + function.getEntryPoint() + " " + function.getName() + " =====");
            DecompileResults result = decompiler.decompileFunction(function, 300, monitor);
            if (result.decompileCompleted()) {
                println(result.getDecompiledFunction().getC());
            } else {
                println("DECOMPILE_FAILED " + result.getErrorMessage());
            }
        }
        decompiler.dispose();
    }
}
