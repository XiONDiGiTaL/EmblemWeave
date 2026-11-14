# Security

The runtime, the tools and the launcher run native code on your PC and read files you give them: game dumps, NSO modules, save data,
shader and texture caches. A bug in how they parse those files, or in how they handle memory, can be a security problem.

If you find one, report it privately through the repository's Security tab ("Report a vulnerability"). Do not open a public issue.
Please include the commit, what you did and what happened.

Things we treat as security problems:

- a crafted NSO, ROM, save or cache file that crashes or runs code in the loader, the runtime or a tool
- the launcher or a setup step writing outside the checkout and the run folder, or writing to the dump
- the recompiler or a tool executing something from an input file

Everything else (crashes with normal files, graphics bugs, wrong behaviour) goes through the normal issues. Only `main` gets fixes.
