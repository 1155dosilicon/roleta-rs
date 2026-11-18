<#
  Compila e instala a Roleta (versão Rust).

  Uso (PowerShell normal, NÃO precisa abrir como administrador):
      powershell -ExecutionPolicy Bypass -File instalar.ps1

  O que faz:
    1. Compila Roleta.exe (cargo build --release).
    2. Pede permissão de administrador (UAC) uma vez e:
         - substitui C:\Program Files\Roleta pelo Roleta.exe novo (um arquivo só)
         - cria a tarefa agendada "Roleta": inicia no logon, como administrador,
           sem pedir UAC (assim a roleta funciona até sobre o Gerenciador de Tarefas)
         - confere que o que a roleta abre roda como usuário normal
         - inicia a roleta
    3. Cria o atalho "Roleta" no Menu Iniciar (dispara a tarefa).

  Config e log ficam em %APPDATA%\Roleta (não são apagados ao reinstalar).
  Desinstalar: powershell -ExecutionPolicy Bypass -File instalar.ps1 -Desinstalar
#>
param(
    [string]$Exe,           # uso interno: Roleta.exe compilado (etapa de administrador)
    [string]$Usuario,       # uso interno
    [string]$Log,           # uso interno
    [switch]$Desinstalar
)

$ErrorActionPreference = 'Stop'
$Destino = Join-Path $env:ProgramFiles 'Roleta'
$Tarefa = 'Roleta'
$ehAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)

# ------------------------------------------------------------ etapa de administrador
if ($ehAdmin -and ($Exe -or $Desinstalar)) {
    if ($Log) { Start-Transcript -Path $Log -Force | Out-Null }
    try {
        Get-Process Roleta -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Seconds 1
        if ($Desinstalar) {
            Unregister-ScheduledTask -TaskName $Tarefa -Confirm:$false -ErrorAction SilentlyContinue
            if (Test-Path $Destino) { Remove-Item $Destino -Recurse -Force }
            Write-Output 'DESINSTALADO'
            return
        }

        # apaga a pasta inteira (inclusive o _internal da versão Python antiga)
        if (Test-Path $Destino) { Remove-Item $Destino -Recurse -Force }
        New-Item -ItemType Directory $Destino | Out-Null
        $exeFinal = Join-Path $Destino 'Roleta.exe'
        Copy-Item $Exe $exeFinal
        Write-Output "Copiado para $Destino"

        $acao  = New-ScheduledTaskAction -Execute $exeFinal -WorkingDirectory $Destino
        $gat   = New-ScheduledTaskTrigger -AtLogOn -User $Usuario
        $princ = New-ScheduledTaskPrincipal -UserId $Usuario -LogonType Interactive -RunLevel Highest
        # sem limite de tempo, roda na bateria, prioridade normal (4) —
        # o padrão (7) deixaria o hook do mouse lento
        $conf  = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) `
                    -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
                    -MultipleInstances IgnoreNew -Priority 4
        Register-ScheduledTask -TaskName $Tarefa -Action $acao -Trigger $gat `
            -Principal $princ -Settings $conf -Force `
            -Description 'Roleta de atalhos do botão lateral do mouse' | Out-Null
        Write-Output 'Tarefa agendada criada'

        # o que a roleta abre deve rodar como usuário normal (nível "Médio")
        $teste = Join-Path $env:TEMP 'roleta_teste_usuario.txt'
        Remove-Item $teste -ErrorAction SilentlyContinue
        Start-Process $exeFinal -ArgumentList "--teste-usuario `"$teste`"" -Wait
        Start-Sleep -Seconds 3
        if (Test-Path $teste) {
            $nivel = (Get-Content $teste | Select-String 'S-1-16-') -replace '\s{2,}.*$', ''
            Write-Output "Programas abertos pela roleta rodam com: $nivel"
            Remove-Item $teste -ErrorAction SilentlyContinue
        } else {
            Write-Output 'AVISO: teste de usuário normal não gerou resultado'
        }

        Start-ScheduledTask -TaskName $Tarefa
        Write-Output 'INSTALADO'
    } catch {
        Write-Output "ERRO: $_"
        exit 1
    } finally {
        if ($Log) { Stop-Transcript | Out-Null }
    }
    return
}

# ------------------------------------------------------------ etapa de usuário
$Aqui = $PSScriptRoot
$logAdmin = Join-Path $env:TEMP 'roleta_instalar.log'
$usuarioAtual = "$env:USERDOMAIN\$env:USERNAME"
$atalhoMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Roleta.lnk'
$atalhoInicio = Join-Path ([Environment]::GetFolderPath('Startup')) 'Roleta.lnk'

if ($Desinstalar) {
    Start-Process powershell -Verb RunAs -Wait -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
        '-Desinstalar', '-Log', "`"$logAdmin`"")
    Remove-Item $atalhoMenu, $atalhoInicio -ErrorAction SilentlyContinue
    Get-Content $logAdmin
    return
}

Write-Output 'Compilando Roleta.exe...'
# o cargo escreve o progresso no stderr: no PowerShell 5.1 isso viraria
# "erro", então o log vai para um arquivo e só o resultado é conferido
$logBuild = Join-Path $env:TEMP 'roleta_build.log'
$p = Start-Process cargo -NoNewWindow -Wait -PassThru -WorkingDirectory $Aqui `
        -RedirectStandardError $logBuild -ArgumentList @('build', '--release')
$compilado = Join-Path $Aqui 'target\release\roleta.exe'
if ($p.ExitCode -ne 0 -or -not (Test-Path $compilado)) {
    Get-Content $logBuild -Tail 15
    throw "Falha ao compilar (log: $logBuild)"
}
# cópia fora da pasta proj para a etapa de administrador
$build = Join-Path $env:TEMP 'roleta_build'
New-Item -ItemType Directory -Force $build | Out-Null
$exeTemp = Join-Path $build 'Roleta.exe'
Copy-Item $compilado $exeTemp -Force
Write-Output ('Compilado ({0:N1} MB).' -f ((Get-Item $exeTemp).Length / 1MB))

Write-Output 'Instalando (vai pedir permissão de administrador)...'
Start-Process powershell -Verb RunAs -Wait -ArgumentList @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
    '-Exe', "`"$exeTemp`"", '-Usuario', "`"$usuarioAtual`"",
    '-Log', "`"$logAdmin`"")
Get-Content $logAdmin | Where-Object { $_ -notmatch '^\*{5,}|^(Início|Fim|Nome|Usu|Execu|Aplica|Máquina|Host|Versão|PS|Build|CLR|WSMan|Serial|Compat|Processo|Iniciar|Transcri|Hora|Configuração)' }

# atalho do Menu Iniciar dispara a tarefa (abre como administrador sem UAC)
$ws = New-Object -ComObject WScript.Shell
$lnk = $ws.CreateShortcut($atalhoMenu)
$lnk.TargetPath = Join-Path $env:SystemRoot 'System32\schtasks.exe'
$lnk.Arguments = "/run /tn `"$Tarefa`""
$lnk.IconLocation = "$Destino\Roleta.exe,0"
$lnk.WindowStyle = 7          # minimizado (não pisca janela)
$lnk.Description = 'Roleta de atalhos do mouse'
$lnk.Save()
# a tarefa agendada já inicia no logon: o atalho antigo da Inicialização sai
Remove-Item $atalhoInicio -ErrorAction SilentlyContinue
Write-Output 'Atalho do Menu Iniciar atualizado.'
