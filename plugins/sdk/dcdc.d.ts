// Type declarations for the dcdc plugin host API.
//
// This is a global script, not a module. The dcdc runtime makes a
// `dcdc` object with this shape available to every plugin module,
// so plugin code uses it without importing anything. Add this file
// to your editor's TypeScript `include` to get completion and
// checking while writing a plugin.

interface Dcdc {
  /** Identity of the plugin running this sub-command. */
  plugin: {
    name: string;
    version: string;
  };

  /** The arguments the user typed after the sub-command. */
  args: string[];

  /**
   * Shell to prefer when running commands in the target: `bash` when
   * present, otherwise `sh`.
   */
  shell: string;

  /** Container this sub-command targets. */
  container: {
    /** Name of the container, or `local` when running on the host. */
    name: string;
    /** True when the target is the host, not a docker container. */
    isLocal: boolean;
    /** Working directory for runs: the bind-mounted cwd when mapped,
     *  otherwise the container's configured working directory. */
    workdir: string;
    /** True when the calling directory is bind-mounted into the
     *  container, so its mise.toml is visible there. */
    cwdMounted: boolean;
  };

  /** Project context. `root` is null when running outside a project. */
  project: {
    root: string | null;
    /** Service names defined by the project's compose file. */
    services: string[];
  };

  /**
   * Run a command on the host (local target) or inside the target
   * container, with stdio inherited. Returns the command's exit
   * code. The second argument is the command's argument list.
   */
  run(cmd: string, args?: string[]): Promise<number>;

  /** Ask the user a yes/no question in the CLI. */
  confirm(message: string): Promise<boolean>;
}

declare const dcdc: Dcdc;
