import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;

import org.hl7.fhir.r5.context.SimpleWorkerContext;
import org.hl7.fhir.r5.formats.IParser;
import org.hl7.fhir.r5.formats.JsonParser;
import org.hl7.fhir.r5.model.StructureMap;
import org.hl7.fhir.r5.utils.structuremap.StructureMapUtilities;

/**
 * Compiles every FML file named on standard input with the HL7 reference compiler. Writes
 * {@code <out>/<n>.json} (or {@code <n>.err}) per file and {@code <out>/index.tsv} mapping each
 * number to its file.
 */
public class CompileAll {
  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    StructureMapUtilities compiler =
        new StructureMapUtilities(new SimpleWorkerContext.SimpleWorkerContextBuilder().fromNothing());
    List<String> files = new BufferedReader(new InputStreamReader(System.in)).lines().toList();
    StringBuilder index = new StringBuilder();
    int ok = 0;
    for (int i = 0; i < files.size(); i++) {
      String file = Paths.get(files.get(i)).toAbsolutePath().toString();
      index.append(i).append('\t').append(file).append('\n');
      try {
        StructureMap map = compiler.parse(Files.readString(Paths.get(file)), file);
        String json = new JsonParser().setOutputStyle(IParser.OutputStyle.PRETTY).composeString(map);
        Files.writeString(out.resolve(i + ".json"), json);
        ok++;
      } catch (Exception e) {
        Files.writeString(out.resolve(i + ".err"), e + "\n");
      }
    }
    Files.writeString(out.resolve("index.tsv"), index.toString());
    System.out.println("compiled " + ok + " of " + files.size());
  }
}
