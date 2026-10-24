"""A text box named like Slack, so Murmur classifies it as work chat. Dumps its contents on exit."""
import sys, tkinter as tk
out = sys.argv[1]
root = tk.Tk(className="Slack"); root.title("general - Slack"); root.geometry("600x200+100+100")
t = tk.Text(root); t.pack(fill="both", expand=True); t.insert("1.0", "Draft: "); t.focus_force()
def dump():
    open(out, "w").write(t.get("1.0", "end-1c")); root.after(500, dump)
root.after(500, dump)
root.mainloop()
