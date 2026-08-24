# Stage plugin code and data for uninstall

When a user chooses to remove configuration data with a plugin, Ora first moves the stopped plugin's installation and data directories into same-volume staging and rolls back if either move fails. The uninstall commits only after both resources are staged, because deleting code and data sequentially would otherwise report success while violating the user's explicit retention choice or leave installed code after destroying its configuration.
