import { ServerIcon, KeyIcon } from 'lucide-react';

export default function LoginPage() {
  // In production, this should point to your actual backend URL.
  // Since we use NGINX to route /auth to the backend, we can just use /auth/login
  const loginUrl = '/auth/login';

  return (
    <div className="min-h-screen bg-slate-50 flex flex-col justify-center py-12 sm:px-6 lg:px-8">
      <div className="sm:mx-auto sm:w-full sm:max-w-md">
        <div className="flex justify-center text-blue-600">
          <ServerIcon className="w-12 h-12" />
        </div>
        <h2 className="mt-6 text-center text-3xl font-extrabold text-slate-900">
          Sys-Manager
        </h2>
        <p className="mt-2 text-center text-sm text-slate-600">
          Secure Multi-System Management
        </p>
      </div>

      <div className="mt-8 sm:mx-auto sm:w-full sm:max-w-md">
        <div className="bg-white py-8 px-4 shadow sm:rounded-lg sm:px-10 border border-slate-200">
          <div className="text-center mb-6">
            <h3 className="text-lg font-medium text-slate-900">Authentication Required</h3>
            <p className="text-sm text-slate-500 mt-1">Please sign in with your authorized GitHub account to access the dashboard.</p>
          </div>
          
          <a
            href={loginUrl}
            className="w-full flex justify-center items-center py-2.5 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-slate-900 hover:bg-slate-800 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-slate-900 transition-colors"
          >
            <KeyIcon className="w-5 h-5 mr-2" />
            Sign in with GitHub
          </a>
        </div>
      </div>
    </div>
  );
}
