import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("lemon");
  if (!config.get<boolean>("server.enabled", true)) {
    // Syntax highlighting only — the grammar contribution needs no client.
    return;
  }

  const command = config.get<string>("server.path", "lemon-lsp");
  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "lemon" }],
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

  // Surface a friendly hint if the server binary is missing, rather than a
  // silent failure.
  client.start().catch((err) => {
    void vscode.window.showWarningMessage(
      `Lemon: could not start language server "${command}". ` +
        `Install it with \`cargo install --path crates/lemon-lsp\` or set ` +
        `\`lemon.server.path\`. (${err})`,
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
