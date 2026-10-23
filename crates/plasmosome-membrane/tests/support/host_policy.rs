pub const HOST_POLICY: &str = "(version 1)
(deny default)
(allow file-read*)
(allow file-write* (subpath \"/usr/local/var/plasmosome/work\"))
(allow network-outbound (remote unix-socket))
(allow mach-lookup)
(allow mach-register)
(allow process-info*)
(allow sysctl-read)
(allow system-socket)
(deny file-write*)
(deny network*)
(deny process-exec)
";
pub const HOST_POLICY_SHA256: &str =
    "d4e3f9a296a4577f54bd51899d3bf6fac439e47e2b559c5df955825d60cd7c34";
pub const WORK_SUBPATH: &str = "(subpath \"/usr/local/var/plasmosome/work\")";
