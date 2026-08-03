import ghidra.app.script.GhidraScript;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeConflictHandler;
import ghidra.program.model.data.DataTypeManager;
import ghidra.program.model.data.DoubleDataType;
import ghidra.program.model.data.FloatDataType;
import ghidra.program.model.data.IntegerDataType;
import ghidra.program.model.data.CharDataType;
import ghidra.program.model.data.PointerDataType;
import ghidra.program.model.data.Structure;
import ghidra.program.model.data.StructureDataType;
import ghidra.program.model.data.UnsignedCharDataType;
import ghidra.program.model.data.UnsignedIntegerDataType;
import ghidra.program.model.data.UnsignedLongDataType;
import ghidra.program.model.data.UnsignedShortDataType;
import ghidra.program.model.data.VoidDataType;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Function.FunctionUpdateType;
import ghidra.program.model.listing.Parameter;
import ghidra.program.model.listing.ParameterImpl;
import ghidra.program.model.symbol.SourceType;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;

public class ApplyRecoveredTypes extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] arguments = getScriptArgs();
        if (arguments.length != 1) {
            throw new IllegalArgumentException("expected SIGNATURE_MANIFEST");
        }
        DataTypeManager types = currentProgram.getDataTypeManager();
        Structure transform = new StructureDataType("SceneViewTransform", 0);
        transform.add(new IntegerDataType(), "viewport_width", null);
        transform.add(new IntegerDataType(), "viewport_height", null);
        transform.add(new DoubleDataType(), "focal_x", null);
        transform.add(new DoubleDataType(), "focal_y", null);
        transform.add(new DoubleDataType(), "scale", null);
        transform.add(new DoubleDataType(), "scene_origin_x", null);
        transform.add(new DoubleDataType(), "scene_origin_y", null);
        transform.add(new DoubleDataType(), "scene_width", null);
        transform.add(new DoubleDataType(), "scene_height", null);
        DataType appliedTransform = types.addDataType(transform, DataTypeConflictHandler.REPLACE_HANDLER);

        Structure point = new StructureDataType("PointF", 0);
        point.add(new DoubleDataType(), "x", null);
        point.add(new DoubleDataType(), "y", null);
        DataType appliedPoint = types.addDataType(point, DataTypeConflictHandler.REPLACE_HANDLER);

        Structure affineTransform = new StructureDataType("AffineTransform", 0);
        affineTransform.add(new DoubleDataType(), "m11", null);
        affineTransform.add(new DoubleDataType(), "m12", null);
        affineTransform.add(new DoubleDataType(), "m13", null);
        affineTransform.add(new DoubleDataType(), "m21", null);
        affineTransform.add(new DoubleDataType(), "m22", null);
        affineTransform.add(new DoubleDataType(), "m23", null);
        affineTransform.add(new DoubleDataType(), "dx", null);
        affineTransform.add(new DoubleDataType(), "dy", null);
        affineTransform.add(new DoubleDataType(), "m33", null);
        affineTransform.add(new UnsignedIntegerDataType(), "type", null);
        affineTransform.add(new UnsignedIntegerDataType(), "dirty", null);
        DataType appliedAffineTransform =
                types.addDataType(affineTransform, DataTypeConflictHandler.REPLACE_HANDLER);

        Structure rawSample = new StructureDataType("RawPenSample", 0);
        rawSample.add(new FloatDataType(), "x", null);
        rawSample.add(new FloatDataType(), "y", null);
        rawSample.add(new FloatDataType(), "pressure", null);
        rawSample.add(new FloatDataType(), "tilt_x", null);
        rawSample.add(new FloatDataType(), "tilt_y", null);
        types.addDataType(rawSample, DataTypeConflictHandler.REPLACE_HANDLER);

        Structure linePoint = new StructureDataType("PackedLinePoint", 14);
        linePoint.replaceAtOffset(0, new FloatDataType(), 4, "x", null);
        linePoint.replaceAtOffset(4, new FloatDataType(), 4, "y", null);
        linePoint.replaceAtOffset(8, new UnsignedShortDataType(), 2, "speed_quarters", null);
        linePoint.replaceAtOffset(10, new UnsignedShortDataType(), 2, "width_quarter_pixels", null);
        linePoint.replaceAtOffset(12, new UnsignedCharDataType(), 1, "direction", null);
        linePoint.replaceAtOffset(13, new UnsignedCharDataType(), 1, "pressure", null);
        types.addDataType(linePoint, DataTypeConflictHandler.REPLACE_HANDLER);

        resetInferredSignature("0091be60");

        DataType appliedRawSample = types.getDataType("/RawPenSample");
        Map<String, DataType> knownTypes = new HashMap<>();
        knownTypes.put("void", new VoidDataType());
        knownTypes.put("f64", new DoubleDataType());
        knownTypes.put("i32", new IntegerDataType());
        knownTypes.put("u32", new UnsignedIntegerDataType());
        knownTypes.put("u64", new UnsignedLongDataType());
        knownTypes.put("ptr", new PointerDataType(new VoidDataType()));
        knownTypes.put("cstr", new PointerDataType(new CharDataType()));
        knownTypes.put("SceneViewTransform*", new PointerDataType(appliedTransform));
        knownTypes.put("RawPenSample*", new PointerDataType(appliedRawSample));
        knownTypes.put("PointF", appliedPoint);
        knownTypes.put("AffineTransform", appliedAffineTransform);

        int appliedSignatures = 0;
        for (String line : Files.readAllLines(Path.of(arguments[0]))) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            String[] fields = line.split("\\t", -1);
            if (fields.length != 3) {
                throw new IllegalArgumentException("invalid signature row: " + line);
            }
            Function function = getFunctionAt(toAddr(fields[0]));
            if (function == null) {
                throw new IllegalStateException("missing function at " + fields[0]);
            }
            String[] specifications = fields[2].isBlank() ? new String[0] : fields[2].split(",", -1);
            Parameter[] parameters = new Parameter[specifications.length];
            for (int index = 0; index < specifications.length; index++) {
                String[] specification = specifications[index].split(":", 2);
                DataType type = requireType(knownTypes, specification[0]);
                parameters[index] = new ParameterImpl(specification[1], type, currentProgram);
            }
            function.replaceParameters(
                    FunctionUpdateType.DYNAMIC_STORAGE_ALL_PARAMS,
                    true,
                    SourceType.USER_DEFINED,
                    parameters);
            function.setReturnType(requireType(knownTypes, fields[1]), SourceType.USER_DEFINED);
            appliedSignatures++;
        }
        println("APPLIED_RECOVERED_TYPES SceneViewTransform PointF AffineTransform RawPenSample PackedLinePoint");
        println("APPLIED_RECOVERED_SIGNATURES " + appliedSignatures);
    }

    private void resetInferredSignature(String address) throws Exception {
        Function function = getFunctionAt(toAddr(address));
        if (function == null) {
            throw new IllegalStateException("missing function at " + address);
        }
        function.replaceParameters(
                FunctionUpdateType.DYNAMIC_STORAGE_ALL_PARAMS,
                true,
                SourceType.ANALYSIS,
                new Parameter[0]);
        function.setReturnType(DataType.DEFAULT, SourceType.ANALYSIS);
    }

    private DataType requireType(Map<String, DataType> knownTypes, String name) {
        DataType type = knownTypes.get(name);
        if (type == null) {
            throw new IllegalArgumentException("unknown recovered type: " + name);
        }
        return type;
    }
}
