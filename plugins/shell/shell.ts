// Pass all of its arguments to the configured container's shell.
//
// `dcdc bash echo "Hello world!"` runs
// `bash -c 'echo "Hello world!"'` in the target container, or on the
// host when the plugin's container is `local`. Each user argument is
// single-quoted so the shell reassembles it as one word, preserving
// what the user's own shell already split for dcdc.

export const name = "shell";
export const version = "0.1.0";
export const description =
  "Run the target container's shell with all arguments as the command line.";
export const aliases = ["bash", "sh"];
export const usage = "dcdc bash <command> [args...]";
export const example = 'dcdc bash echo "Hello world!"';

export default async (args: string[]): Promise<number> => {
  const quoted = args
    .map((arg) => `'${arg.replaceAll("'", "'\\''")}'`)
    .join(" ");
  return dcdc.run(dcdc.shell, ["-c", quoted]);
};
