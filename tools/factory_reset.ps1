param (
    [string]$LOCALE="en"
)
Write-Output "This script will do a factory reset, removing all stored data and keys."
$title    = 'Factory reset'
$question = 'Are you sure you want to proceed?'
$choices  = '&Yes', '&No'

$decision = $Host.UI.PromptForChoice($title, $question, $choices, 1)
if ($decision -eq 0) {
    Write-Host 'confirmed'
} else {
    Exit
}

python usb_update.py --disable-boot
Write-Output "waiting for device to reboot"
Start-Sleep 5

Invoke-WebRequest https://ci.betrusted.io/releases/latest/xous-$LOCALE.img -OutFile xous.img
python usb_update.py -k xous.img
Remove-Item xous.img

Write-Output "waiting for device to reboot"
Start-Sleep 5

Invoke-WebRequest https://ci.betrusted.io/releases/latest/ec_fw.bin -OutFile ec_fw.bin
python usb_update.py -e ec_fw.bin
Remove-Item ec_fw.bin

Write-Output "waiting for device to reboot"
Start-Sleep 5

Invoke-WebRequest https://ci.betrusted.io/releases/latest/wf200_fw.bin -OutFile wf200_fw.bin
python usb_update.py -w wf200_fw.bin
Remove-Item wf200_fw.bin

Write-Output "waiting for device to reboot"
Start-Sleep 5

Invoke-WebRequest https://ci.betrusted.io/releases/latest/soc_csr.bin -OutFile soc_csr.bin
Invoke-WebRequest https://ci.betrusted.io/releases/latest/loader.bin -OutFile loader.bin
python usb_update.py --enable-boot --soc soc_csr.bin -l loader.bin
Remove-Item soc_csr.bin
Remove-Item loader.bin

Write-Output "Please insert a paperclip in the hard reset hole in the lower right hand corner to ensure the new FPGA gateware is loaded."
Write-Output "After inserting the paperclip you will need to apply power via USB to boot."
Write-Output "IMPORTANT: you must run 'ecup auto' to update the EC with the staged firmware objects."
