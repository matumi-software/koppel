try { new ActiveXObject("WScript.Shell").RegRead("HKLM\SOFTWARE\Matumi\Koppel\DriverRestartRequired"); Session.Property("DRIVER_RESTART") = "1"; } catch (e) { }
