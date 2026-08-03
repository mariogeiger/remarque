import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;

public class DecompileFunctionsInRange extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 2) {
            throw new IllegalArgumentException("expected START END");
        }
        Address start = toAddr(arguments[0]);
        Address end = toAddr(arguments[1]);
        DecompInterface decompiler = new DecompInterface();
        decompiler.openProgram(currentProgram);
        FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(start, true);
        while (functions.hasNext()) {
            Function function = functions.next();
            if (function.getEntryPoint().compareTo(end) >= 0) {
                break;
            }
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
