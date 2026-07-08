import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { resolveServer } from "./server";

let client: LanguageClient | undefined;

export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  const config = vscode.workspace.getConfiguration("lemon");
  if (!config.get<boolean>("server.enabled", true)) {
    // Syntax highlighting only — the grammar contribution needs no client.
    return;
  }

  const output = vscode.window.createOutputChannel("Lemon");
  context.subscriptions.push(output);

  const command = await resolveServer(context, config, output);
  if (!command) {
    void vscode.window.showWarningMessage(
      "Lemon: no language server available (syntax highlighting still works). " +
        "Install it with `cargo install --path crates/lemon-lsp`, set " +
        "`lemon.server.path`, or enable `lemon.server.autoDownload`.",
    );
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "lemon" }],
    outputChannel: output,
    // Supply the engine's known series names so the server can flag unknown /
    // typo'd series. Empty by default (that check is then skipped).
    initializationOptions: {
      series: config.get<string[]>("series", []),
    },
  };

  client = new LanguageClient(
    "lemon",
    "Lemon Language Server",
    serverOptions,
    clientOptions,
  );

  client.start().catch((err) => {
    void vscode.window.showWarningMessage(
      `Lemon: could not start language server "${command}". (${err})`,
    );
  });

  context.subscriptions.push({
    dispose: () => {
      void client?.stop();
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
