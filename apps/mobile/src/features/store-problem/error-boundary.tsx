// SPDX-License-Identifier: Apache-2.0
// The last net above the whole app: a screen that throws while rendering shows
// "couldn't load" with "Try again" instead of an empty page. A retry that
// throws again comes back to the screen with "still didn't load".
import { Component, type ReactNode } from "react";
import { ipc } from "../../ipc";
import { LoadFailed } from "./store-problem-screen";

type State = { failed: boolean; retried: boolean };

export class AppErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { failed: false, retried: false };

  static getDerivedStateFromError(): Partial<State> {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    // For the diagnostics log: the error's type only, never its message.
    const kind = error instanceof Error ? error.name : "Error";
    void Promise.resolve(ipc.commands.logUiFailure(kind)).catch(() => undefined);
  }

  render() {
    return this.state.failed ? (
      <LoadFailed already={this.state.retried} onRetry={() => this.setState({ failed: false, retried: true })} />
    ) : (
      this.props.children
    );
  }
}
