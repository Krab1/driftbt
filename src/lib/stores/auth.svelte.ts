import { remote } from "../api";
import { checkToken, forgetToken, saveToken, setUnauthorizedHandler, storedToken } from "../api/http";

/** Only the remote web UI needs a token; the desktop app and the demo mock are always "in". */
class Auth {
  state = $state<"checking" | "in" | "out">(remote ? "checking" : "in");

  async start() {
    if (!remote) return;
    setUnauthorizedHandler(() => (this.state = "out"));
    this.state = storedToken() && (await checkToken()) ? "in" : "out";
  }

  /** Resolves to true when the server accepted the token. */
  async login(token: string, remember: boolean): Promise<boolean> {
    const t = token.trim();
    if (!t || !(await checkToken(t))) return false;
    saveToken(t, remember);
    this.state = "in";
    return true;
  }

  logout() {
    forgetToken();
    this.state = "out";
  }
}

export const auth = new Auth();
