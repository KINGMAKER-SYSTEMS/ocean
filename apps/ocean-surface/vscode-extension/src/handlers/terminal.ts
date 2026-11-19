import * as vscode from "vscode";
import { spawn, type ChildProcess } from "node:child_process";
import { StringDecoder } from "node:string_decoder";
import type {
  CreateTerminalRequest,
  CreateTerminalResponse,
  KillTerminalRequest,
  KillTerminalResponse,
  ReleaseTerminalRequest,
  ReleaseTerminalResponse,
  TerminalOutputRequest,
  TerminalOutputResponse,
  WaitForTerminalExitRequest,
  WaitForTerminalExitResponse,
} from "@agentclientprotocol/sdk";
import { log, logError } from "../logger";

interface ManagedTerminal {
  id: string;
  process: ChildProcess;
  output: string;
  truncated: boolean;
  outputByteLimit: number;
  exitCode: number | null;
  exitSignal: string | null;
  exited: boolean;
  exitPromise: Promise<void>;
}

export class TerminalHandler {
  private terminals = new Map<string, ManagedTerminal>();
  private nextId = 1;

  async createTerminal(params: CreateTerminalRequest): Promise<CreateTerminalResponse> {
    const terminalId = `term_${this.nextId++}`;
    const outputByteLimit = params.outputByteLimit ?? 1024 * 1024;
    log(
      `createTerminal: ${params.command} ${(params.args ?? []).join(" ")} (id=${terminalId})`,
    );

    const env: Record<string, string> = { ...(process.env as Record<string, string>) };
    if (params.env) {
      for (const entry of params.env) {
        env[entry.name] = entry.value;
      }
    }

    const child = spawn(params.command, params.args ?? [], {
      cwd: params.cwd || undefined,
      env,
      shell: false,
      stdio: ["pipe", "pipe", "pipe"],
    });

    const writeEmitter = new vscode.EventEmitter<string>();
    const pty: vscode.Pseudoterminal = {
      onDidWrite: writeEmitter.event,
      open() {
        writeEmitter.fire(`$ ${params.command} ${(params.args ?? []).join(" ")}\r\n`);
      },
      close() {},
    };
    vscode.window.createTerminal({ name: `Ocean: ${params.command}`, pty });

    let resolveExit: () => void;
    const exitPromise = new Promise<void>((resolve) => {
      resolveExit = resolve;
    });

    const managed: ManagedTerminal = {
      id: terminalId,
      process: child,
      output: "",
      truncated: false,
      outputByteLimit,
      exitCode: null,
      exitSignal: null,
      exited: false,
      exitPromise,
    };
    this.terminals.set(terminalId, managed);

    const appendOutput = (text: string) => {
      writeEmitter.fire(text.replace(/\n/g, "\r\n"));
      managed.output += text;
      const trimmed = trimToByteLimit(managed.output, managed.outputByteLimit);
      managed.output = trimmed.output;
      managed.truncated = managed.truncated || trimmed.truncated;
    };

    for (const stream of [child.stdout, child.stderr]) {
      const decoder = new StringDecoder("utf8");
      stream?.on("data", (data: Buffer) => appendOutput(decoder.write(data)));
      stream?.on("end", () => appendOutput(decoder.end()));
    }

    child.on("close", (code, signal) => {
      managed.exitCode = code;
      managed.exitSignal = signal;
      managed.exited = true;
      resolveExit();
    });
    child.on("error", (error) => {
      appendOutput(`${error.message}\n`);
      managed.exited = true;
      resolveExit();
    });

    return { terminalId };
  }

  snapshot(terminalId: string): TerminalOutputResponse | null {
    const managed = this.terminals.get(terminalId);
    if (!managed) {
      return null;
    }
    const response: TerminalOutputResponse = {
      output: managed.output,
      truncated: managed.truncated,
    };
    if (managed.exited) {
      response.exitStatus = {
        exitCode: managed.exitCode,
        signal: managed.exitSignal,
      };
    }
    return response;
  }

  async terminalOutput(params: TerminalOutputRequest): Promise<TerminalOutputResponse> {
    const response = this.snapshot(params.terminalId);
    if (!response) {
      throw new Error(`Terminal not found: ${params.terminalId}`);
    }
    return response;
  }

  async waitForTerminalExit(
    params: WaitForTerminalExitRequest,
  ): Promise<WaitForTerminalExitResponse> {
    const managed = this.terminals.get(params.terminalId);
    if (!managed) {
      throw new Error(`Terminal not found: ${params.terminalId}`);
    }
    await managed.exitPromise;
    return { exitCode: managed.exitCode, signal: managed.exitSignal };
  }

  async killTerminal(params: KillTerminalRequest): Promise<KillTerminalResponse> {
    const managed = this.terminals.get(params.terminalId);
    if (!managed) {
      throw new Error(`Terminal not found: ${params.terminalId}`);
    }
    try {
      managed.process.kill("SIGTERM");
    } catch (error) {
      logError(`Failed to kill terminal ${params.terminalId}`, error);
    }
    return {};
  }

  async releaseTerminal(params: ReleaseTerminalRequest): Promise<ReleaseTerminalResponse> {
    const managed = this.terminals.get(params.terminalId);
    if (!managed) {
      throw new Error(`Terminal not found: ${params.terminalId}`);
    }
    if (!managed.exited) {
      try {
        managed.process.kill("SIGTERM");
      } catch {
        // ignore
      }
    }
    this.terminals.delete(params.terminalId);
    return {};
  }

  dispose(): void {
    for (const managed of this.terminals.values()) {
      try {
        if (!managed.exited) {
          managed.process.kill("SIGKILL");
        }
      } catch {
        // ignore
      }
    }
    this.terminals.clear();
  }
}

function trimToByteLimit(
  output: string,
  outputByteLimit: number,
): { output: string; truncated: boolean } {
  if (Buffer.byteLength(output, "utf-8") <= outputByteLimit) {
    return { output, truncated: false };
  }

  let bytes = 0;
  const retained: string[] = [];
  for (const char of Array.from(output).reverse()) {
    const charBytes = Buffer.byteLength(char, "utf-8");
    if (bytes + charBytes > outputByteLimit) {
      break;
    }
    retained.push(char);
    bytes += charBytes;
  }

  return { output: retained.reverse().join(""), truncated: true };
}
