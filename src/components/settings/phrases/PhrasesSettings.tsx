import React, { useCallback, useEffect, useState } from "react";
import { Plus, Pencil, Trash2, Check, X } from "lucide-react";
import { commands, type PhraseRecord } from "../../../bindings";
import { Button } from "../../ui/Button";
import { Input } from "../../ui/Input";
import { Textarea } from "../../ui/Textarea";
import { SettingsGroup } from "../../ui/SettingsGroup";

// DotFlow — the editable phrase library (design §8). Beeftext-simple: a trigger → text-block list, but the
// trigger fires as you SPEAK during dictation (say the spoken trigger, or type the dot trigger). Every edit
// here rebuilds the table the live dictation reads, so a new phrase works on the very next utterance.

interface Draft {
  key: string;
  aliasesText: string; // comma-separated in the UI
  expansion: string;
}

const emptyDraft: Draft = { key: "", aliasesText: "", expansion: "" };

const parseAliases = (text: string): string[] =>
  text
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

export const PhrasesSettings: React.FC = () => {
  const [phrases, setPhrases] = useState<PhraseRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // null = not editing; "new" = adding; number = editing that id.
  const [editing, setEditing] = useState<number | "new" | null>(null);
  const [draft, setDraft] = useState<Draft>(emptyDraft);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    const res = await commands.getPhrases();
    if (res.status === "ok") {
      setPhrases(res.data);
      setError(null);
    } else {
      setError(res.error);
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const startAdd = () => {
    setDraft(emptyDraft);
    setEditing("new");
  };

  const startEdit = (p: PhraseRecord) => {
    setDraft({ key: p.key, aliasesText: p.aliases.join(", "), expansion: p.expansion });
    setEditing(p.id);
  };

  const cancel = () => {
    setEditing(null);
    setDraft(emptyDraft);
  };

  const canSave =
    draft.expansion.trim().length > 0 &&
    (draft.key.trim().length > 0 || parseAliases(draft.aliasesText).length > 0);

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    const key = draft.key.trim().replace(/^\./, ""); // tolerate a leading dot the user typed
    const aliases = parseAliases(draft.aliasesText);
    const expansion = draft.expansion;
    const res =
      editing === "new"
        ? await commands.addPhrase(key, aliases, expansion)
        : await commands.updatePhrase(editing as number, key, aliases, expansion);
    setSaving(false);
    if (res.status === "ok") {
      cancel();
      await load();
    } else {
      setError(res.error);
    }
  };

  const remove = async (id: number) => {
    const res = await commands.deletePhrase(id);
    if (res.status === "ok") {
      if (editing === id) cancel();
      await load();
    } else {
      setError(res.error);
    }
  };

  const editor = (
    <div className="flex flex-col gap-3 p-3 rounded-lg border border-logo-primary/40 bg-logo-primary/5">
      <div className="flex flex-col gap-1">
        <label className="text-xs opacity-70">Dot trigger — type it as .key (optional)</label>
        <Input
          variant="compact"
          placeholder="fu"
          value={draft.key}
          onChange={(e) => setDraft({ ...draft, key: e.target.value })}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs opacity-70">
          Spoken triggers — say any of these during dictation (comma-separated)
        </label>
        <Input
          variant="compact"
          placeholder="insert follow up, insert fu"
          value={draft.aliasesText}
          onChange={(e) => setDraft({ ...draft, aliasesText: e.target.value })}
        />
      </div>
      <div className="flex flex-col gap-1">
        <label className="text-xs opacity-70">Inserts this text</label>
        <Textarea
          variant="compact"
          placeholder="Following up on this — let me know if you need anything else."
          value={draft.expansion}
          onChange={(e) => setDraft({ ...draft, expansion: e.target.value })}
        />
      </div>
      <div className="flex gap-2 justify-end">
        <Button variant="ghost" size="sm" onClick={cancel} disabled={saving}>
          <span className="flex items-center gap-1">
            <X size={14} /> Cancel
          </span>
        </Button>
        <Button variant="primary" size="sm" onClick={save} disabled={!canSave || saving}>
          <span className="flex items-center gap-1">
            <Check size={14} /> {saving ? "Saving…" : "Save"}
          </span>
        </Button>
      </div>
    </div>
  );

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title="Phrase library">
        <div className="flex flex-col gap-3 p-1">
          <p className="text-sm opacity-70">
            Create your own dictation inserts: a trigger you speak (or a dot shortcut you type) drops in a
            saved block of text. Edits take effect on your next dictation — no restart.
          </p>

          {error && (
            <div className="text-sm text-red-400 bg-red-500/10 rounded-md px-3 py-2">{error}</div>
          )}

          {editing === "new" ? (
            editor
          ) : (
            <div>
              <Button variant="primary-soft" size="sm" onClick={startAdd}>
                <span className="flex items-center gap-1">
                  <Plus size={14} /> New phrase
                </span>
              </Button>
            </div>
          )}

          {loading ? (
            <p className="text-sm opacity-60">Loading…</p>
          ) : phrases.length === 0 && editing !== "new" ? (
            <p className="text-sm opacity-60">No phrases yet — add your first one above.</p>
          ) : (
            <div className="flex flex-col gap-2">
              {phrases.map((p) =>
                editing === p.id ? (
                  <div key={p.id}>{editor}</div>
                ) : (
                  <div
                    key={p.id}
                    className="flex items-start gap-3 p-3 rounded-lg border border-mid-gray/20 hover:border-mid-gray/40"
                  >
                    <div className="flex-1 min-w-0">
                      <div className="flex flex-wrap items-center gap-1.5 mb-1">
                        {p.key && (
                          <span className="text-xs font-mono px-1.5 py-0.5 rounded bg-logo-primary/20">
                            .{p.key}
                          </span>
                        )}
                        {p.aliases.map((a) => (
                          <span
                            key={a}
                            className="text-xs px-1.5 py-0.5 rounded bg-mid-gray/15 opacity-80"
                            title="spoken trigger"
                          >
                            “{a}”
                          </span>
                        ))}
                      </div>
                      <p className="text-sm opacity-80 whitespace-pre-wrap break-words line-clamp-3">
                        {p.expansion}
                      </p>
                    </div>
                    <div className="flex gap-1 shrink-0">
                      <Button variant="ghost" size="sm" onClick={() => startEdit(p)} title="Edit">
                        <Pencil size={14} />
                      </Button>
                      <Button
                        variant="danger-ghost"
                        size="sm"
                        onClick={() => remove(p.id)}
                        title="Delete"
                      >
                        <Trash2 size={14} />
                      </Button>
                    </div>
                  </div>
                ),
              )}
            </div>
          )}
        </div>
      </SettingsGroup>
    </div>
  );
};
