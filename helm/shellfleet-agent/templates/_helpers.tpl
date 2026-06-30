{{/*
Expand the name of the chart.
*/}}
{{- define "shellfleet-agent.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/*
Fully qualified app name — release-name + chart-name unless overridden.
*/}}
{{- define "shellfleet-agent.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "shellfleet-agent.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "shellfleet-agent.labels" -}}
helm.sh/chart: {{ include "shellfleet-agent.chart" . }}
{{ include "shellfleet-agent.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "shellfleet-agent.selectorLabels" -}}
app.kubernetes.io/name: {{ include "shellfleet-agent.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "shellfleet-agent.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "shellfleet-agent.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- required "serviceAccount.name is required when serviceAccount.create=false; refusing to bind cluster privileges to the namespace default account" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}
